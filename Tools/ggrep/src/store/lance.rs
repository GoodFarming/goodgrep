//! LanceDB-backed vector storage with Arrow integration.

use std::{
   collections::{HashMap, HashSet, hash_map::Entry},
   fs,
   path::{Path, PathBuf},
   sync::Arc,
};

use arrow_array::{
   Array, FixedSizeListArray, Float32Array, Float64Array, LargeBinaryArray, LargeStringArray,
   RecordBatch, RecordBatchReader, StringArray, UInt32Array,
   builder::{
      BinaryBuilder, Float32Builder, Float64Builder, LargeBinaryBuilder, LargeStringBuilder,
      StringBuilder, UInt32Builder,
   },
};
use arrow_schema::{ArrowError, DataType, Field, Schema, SchemaRef};
use futures::TryStreamExt;
use lancedb::{
   Connection, Table, connect,
   index::{Index, scalar::FullTextSearchQuery},
   query::{ExecutableQuery, QueryBase},
};
use parking_lot::RwLock;

use crate::{
   config,
   error::Result,
   search::colbert::max_sim_quantized,
   store,
   types::{ChunkType, SearchResponse, SearchResult, SearchStatus, VectorRecord},
   util::probe_store_path,
};

/// Errors that can occur during `LanceDB` operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
   #[error("invalid database path")]
   InvalidDatabasePath,

   #[error("failed to connect to database: {0}")]
   Connect(#[source] lancedb::Error),

   #[error("failed to reopen table after migration: {0}")]
   ReopenTableAfterMigration(#[source] lancedb::Error),

   #[error("failed to create table: {0}")]
   CreateTable(#[source] lancedb::Error),

   #[error("failed to sample table for migration check: {0}")]
   SampleTableForMigration(#[source] lancedb::Error),

   #[error("failed to read sample batch: {0}")]
   ReadSampleBatch(#[source] lancedb::Error),

   #[error("failed to read existing data for migration: {0}")]
   ReadExistingDataForMigration(#[source] lancedb::Error),

   #[error("failed to collect existing batches: {0}")]
   CollectExistingBatches(#[source] lancedb::Error),

   #[error("failed to drop old table during migration: {0}")]
   DropOldTableDuringMigration(#[source] lancedb::Error),

   #[error("failed to create new table during migration: {0}")]
   CreateNewTableDuringMigration(#[source] lancedb::Error),

   #[error("failed to add migrated records: {0}")]
   AddMigratedRecords(#[source] lancedb::Error),

   #[error("failed to create empty batch: {0}")]
   CreateEmptyBatch(#[source] ArrowError),

   #[error("empty batch")]
   EmptyBatch,

   #[error("failed to create record batch: {0}")]
   CreateRecordBatch(#[source] ArrowError),

   #[error("failed to add records: {0}")]
   AddRecords(#[source] lancedb::Error),

   #[error("failed to create vector query: {0}")]
   CreateVectorQuery(#[source] lancedb::Error),

   #[error("failed to execute code search: {0}")]
   ExecuteCodeSearch(#[source] lancedb::Error),

   #[error("failed to execute doc search: {0}")]
   ExecuteDocSearch(#[source] lancedb::Error),

   #[error("failed to collect code results: {0}")]
   CollectCodeResults(#[source] lancedb::Error),

   #[error("failed to collect doc results: {0}")]
   CollectDocResults(#[source] lancedb::Error),

   #[error("missing path column")]
   MissingPathColumn,

   #[error("path column type mismatch")]
   PathColumnTypeMismatch,

   #[error("missing start_line column")]
   MissingStartLineColumn,

   #[error("start_line type mismatch")]
   StartLineTypeMismatch,

   #[error("content column type mismatch")]
   ContentColumnTypeMismatch,

   #[error("vector column type mismatch")]
   VectorColumnTypeMismatch,

   #[error("vector values type mismatch")]
   VectorValuesTypeMismatch,

   #[error("failed to delete file: {0}")]
   DeleteFile(#[source] lancedb::Error),

   #[error("failed to delete files: {0}")]
   DeleteFiles(#[source] lancedb::Error),

   #[error("failed to drop table: {0}")]
   DropTable(#[source] lancedb::Error),

   #[error("failed to count rows: {0}")]
   CountRows(#[source] lancedb::Error),

   #[error("failed to execute query: {0}")]
   ExecuteQuery(#[source] lancedb::Error),

   #[error("failed to collect results: {0}")]
   CollectResults(#[source] lancedb::Error),

   #[error("failed to list tables: {0}")]
   ListTables(#[source] lancedb::Error),

   #[error("index already exists")]
   IndexAlreadyExists,

   #[error("failed to create FTS index: {0}")]
   CreateFtsIndex(#[source] lancedb::Error),

   #[error("failed to create vector index: {0}")]
   CreateVectorIndex(#[source] lancedb::Error),
}

/// Single-use [`RecordBatch`] iterator for `LanceDB` table creation.
pub enum RecordBatchOnce {
   Batch(RecordBatch),
   Taken(SchemaRef),
   __Invalid,
}

impl RecordBatchOnce {
   pub const fn new(batch: RecordBatch) -> Self {
      Self::Batch(batch)
   }

   /// Extracts the batch if available, returning its schema if already taken.
   pub fn take(&mut self) -> Result<RecordBatch, SchemaRef> {
      let prev = std::mem::replace(self, Self::__Invalid);
      match prev {
         Self::Batch(batch) => {
            *self = Self::Taken(batch.schema());
            Ok(batch)
         },
         Self::Taken(schema) => {
            *self = Self::Taken(schema.clone());
            Err(schema)
         },
         Self::__Invalid => {
            unreachable!()
         },
      }
   }
}

impl Iterator for RecordBatchOnce {
   type Item = Result<RecordBatch, ArrowError>;

   fn next(&mut self) -> Option<Self::Item> {
      self.take().ok().map(Ok)
   }
}

impl RecordBatchReader for RecordBatchOnce {
   fn schema(&self) -> arrow_schema::SchemaRef {
      match self {
         Self::Batch(batch) => batch.schema(),
         Self::Taken(schema) => schema.clone(),
         Self::__Invalid => unreachable!(),
      }
   }
}

/// `LanceDB` store with connection pooling for per-segment tables.
pub struct LanceStore {
   connections: RwLock<HashMap<String, Arc<Connection>>>,
   data_dir:    PathBuf,
}

impl LanceStore {
   /// Creates a new store using the data directory from configuration.
   pub fn new() -> Result<Self> {
      let data_dir = config::data_dir();
      fs::create_dir_all(data_dir)?;
      probe_store_path(data_dir)?;

      Ok(Self { connections: RwLock::new(HashMap::new()), data_dir: data_dir.clone() })
   }

   async fn get_connection(&self, store_id: &str) -> Result<Arc<Connection>> {
      {
         let connections = self.connections.read();
         if let Some(conn) = connections.get(store_id) {
            return Ok(Arc::clone(conn));
         }
      }

      let db_path = self.data_dir.join(store_id);
      tokio::fs::create_dir_all(&db_path).await?;

      let conn = connect(db_path.to_str().ok_or(StoreError::InvalidDatabasePath)?)
         .execute()
         .await
         .map_err(StoreError::Connect)?;

      let conn = Arc::new(conn);

      let mut connections = self.connections.write();
      match connections.entry(store_id.to_string()) {
         Entry::Occupied(e) => Ok(Arc::clone(e.get())),
         Entry::Vacant(e) => {
            e.insert(Arc::clone(&conn));
            Ok(conn)
         },
      }
   }

   pub(crate) async fn get_table(&self, store_id: &str, table_name: &str) -> Result<Table> {
      let conn = self.get_connection(store_id).await?;

      let table = if let Ok(table) = conn.open_table(table_name).execute().await {
         Self::check_and_migrate_table(&conn, table_name, &table).await?;
         conn
            .open_table(table_name)
            .execute()
            .await
            .map_err(StoreError::ReopenTableAfterMigration)?
      } else {
         let schema = Self::create_schema();
         let empty_batch = Self::create_empty_batch(&schema)?;

         conn
            .create_table(table_name, RecordBatchOnce::new(empty_batch))
            .execute()
            .await
            .map_err(StoreError::CreateTable)?
      };
      Ok(table)
   }

   async fn check_and_migrate_table(
      conn: &Connection,
      table_name: &str,
      table: &Table,
   ) -> Result<()> {
      let _ = (conn, table_name, table);
      Ok(())
   }

   fn create_schema() -> Arc<Schema> {
      Arc::new(Schema::new(vec![
         Field::new("row_id", DataType::Utf8, false),
         Field::new("chunk_id", DataType::Utf8, false),
         Field::new("path_key", DataType::Utf8, false),
         Field::new("path_key_ci", DataType::Utf8, false),
         Field::new("ordinal", DataType::UInt32, false),
         Field::new("file_hash", DataType::Binary, false),
         Field::new("chunk_hash", DataType::Binary, false),
         Field::new("chunker_version", DataType::Utf8, false),
         Field::new("kind", DataType::Utf8, false),
         Field::new("text", DataType::LargeUtf8, false),
         Field::new("start_line", DataType::UInt32, true),
         Field::new("end_line", DataType::UInt32, true),
         Field::new(
            "embedding",
            DataType::FixedSizeList(
               Arc::new(Field::new("item", DataType::Float32, true)),
               config::get().dense_dim as i32,
            ),
            false,
         ),
         Field::new("colbert", DataType::LargeBinary, true),
         Field::new("colbert_scale", DataType::Float64, true),
         Field::new("chunk_type", DataType::Utf8, true),
         Field::new("context_prev", DataType::Utf8, true),
         Field::new("context_next", DataType::Utf8, true),
      ]))
   }

   fn create_empty_batch(schema: &Arc<Schema>) -> Result<RecordBatch> {
      let row_id_array = StringBuilder::new().finish();
      let chunk_id_array = StringBuilder::new().finish();
      let path_key_array = StringBuilder::new().finish();
      let path_key_ci_array = StringBuilder::new().finish();
      let ordinal_array = UInt32Builder::new().finish();
      let file_hash_array = BinaryBuilder::new().finish();
      let chunk_hash_array = BinaryBuilder::new().finish();
      let chunker_array = StringBuilder::new().finish();
      let kind_array = StringBuilder::new().finish();
      let text_array = LargeStringBuilder::new().finish();
      let start_line_array = UInt32Builder::new().finish();
      let end_line_array = UInt32Builder::new().finish();

      let vector_values = Float32Builder::new().finish();
      let vector_array = FixedSizeListArray::new(
         Arc::new(Field::new("item", DataType::Float32, true)),
         config::get().dense_dim as i32,
         Arc::new(vector_values),
         None,
      );

      let colbert_array = LargeBinaryBuilder::new().finish();
      let colbert_scale_array = Float64Builder::new().finish();
      let chunk_type_array = StringBuilder::new().finish();
      let context_prev_array = StringBuilder::new().finish();
      let context_next_array = StringBuilder::new().finish();

      Ok(RecordBatch::try_new(schema.clone(), vec![
         Arc::new(row_id_array),
         Arc::new(chunk_id_array),
         Arc::new(path_key_array),
         Arc::new(path_key_ci_array),
         Arc::new(ordinal_array),
         Arc::new(file_hash_array),
         Arc::new(chunk_hash_array),
         Arc::new(chunker_array),
         Arc::new(kind_array),
         Arc::new(text_array),
         Arc::new(start_line_array),
         Arc::new(end_line_array),
         Arc::new(vector_array),
         Arc::new(colbert_array),
         Arc::new(colbert_scale_array),
         Arc::new(chunk_type_array),
         Arc::new(context_prev_array),
         Arc::new(context_next_array),
      ])
      .map_err(StoreError::CreateEmptyBatch)?)
   }

   fn records_to_batch(records: Vec<VectorRecord>) -> Result<RecordBatch> {
      if records.is_empty() {
         return Err(StoreError::EmptyBatch.into());
      }

      let cfg = config::get();
      let schema = Self::create_schema();
      let _len = records.len();

      let mut row_id_builder = StringBuilder::new();
      let mut chunk_id_builder = StringBuilder::new();
      let mut path_key_builder = StringBuilder::new();
      let mut path_key_ci_builder = StringBuilder::new();
      let mut ordinal_builder = UInt32Builder::new();
      let mut file_hash_builder = BinaryBuilder::new();
      let mut chunk_hash_builder = BinaryBuilder::new();
      let mut chunker_builder = StringBuilder::new();
      let mut kind_builder = StringBuilder::new();
      let mut text_builder = LargeStringBuilder::new();
      let mut start_line_builder = UInt32Builder::new();
      let mut end_line_builder = UInt32Builder::new();
      let mut vector_builder = Float32Builder::new();
      let mut colbert_builder = LargeBinaryBuilder::new();
      let mut colbert_scale_builder = Float64Builder::new();
      let mut chunk_type_builder = StringBuilder::new();
      let mut context_prev_builder = StringBuilder::new();
      let mut context_next_builder = StringBuilder::new();

      let dim = cfg.dense_dim;
      for record in records {
         row_id_builder.append_value(&record.row_id);
         chunk_id_builder.append_value(&record.chunk_id);
         path_key_builder.append_value(store::path_to_store_value(&record.path_key));
         path_key_ci_builder.append_value(&record.path_key_ci);
         ordinal_builder.append_value(record.ordinal);
         file_hash_builder.append_value(record.file_hash);
         chunk_hash_builder.append_value(record.chunk_hash);
         chunker_builder.append_value(&record.chunker);
         kind_builder.append_value(&record.kind);
         text_builder.append_value(&record.text);
         start_line_builder.append_value(record.start_line);
         end_line_builder.append_value(record.end_line);

         if record.vector.len() != dim {
            return Err(StoreError::VectorColumnTypeMismatch.into());
         }

         for &val in &record.vector {
            vector_builder.append_value(val);
         }

         colbert_builder.append_value(&record.colbert);
         colbert_scale_builder.append_value(record.colbert_scale);

         if let Some(chunk_type) = record.chunk_type {
            chunk_type_builder.append_value(chunk_type.as_lowercase_str());
         } else {
            chunk_type_builder.append_null();
         }

         if let Some(prev) = &record.context_prev {
            context_prev_builder.append_value(prev);
         } else {
            context_prev_builder.append_null();
         }

         if let Some(next) = &record.context_next {
            context_next_builder.append_value(next);
         } else {
            context_next_builder.append_null();
         }
      }

      let row_id_array = row_id_builder.finish();
      let chunk_id_array = chunk_id_builder.finish();
      let path_key_array = path_key_builder.finish();
      let path_key_ci_array = path_key_ci_builder.finish();
      let ordinal_array = ordinal_builder.finish();
      let file_hash_array = file_hash_builder.finish();
      let chunk_hash_array = chunk_hash_builder.finish();
      let chunker_array = chunker_builder.finish();
      let kind_array = kind_builder.finish();
      let text_array = text_builder.finish();
      let start_line_array = start_line_builder.finish();
      let end_line_array = end_line_builder.finish();

      let vector_values_array = vector_builder.finish();
      let vector_array = FixedSizeListArray::new(
         Arc::new(Field::new("item", DataType::Float32, true)),
         dim as i32,
         Arc::new(vector_values_array),
         None,
      );

      let colbert_array = colbert_builder.finish();
      let colbert_scale_array = colbert_scale_builder.finish();
      let chunk_type_array = chunk_type_builder.finish();
      let context_prev_array = context_prev_builder.finish();
      let context_next_array = context_next_builder.finish();

      Ok(RecordBatch::try_new(schema, vec![
         Arc::new(row_id_array),
         Arc::new(chunk_id_array),
         Arc::new(path_key_array),
         Arc::new(path_key_ci_array),
         Arc::new(ordinal_array),
         Arc::new(file_hash_array),
         Arc::new(chunk_hash_array),
         Arc::new(chunker_array),
         Arc::new(kind_array),
         Arc::new(text_array),
         Arc::new(start_line_array),
         Arc::new(end_line_array),
         Arc::new(vector_array),
         Arc::new(colbert_array),
         Arc::new(colbert_scale_array),
         Arc::new(chunk_type_array),
         Arc::new(context_prev_array),
         Arc::new(context_next_array),
      ])
      .map_err(StoreError::CreateRecordBatch)?)
   }

   fn parse_chunk_type(s: &str) -> ChunkType {
      match s {
         "function" => ChunkType::Function,
         "class" => ChunkType::Class,
         "interface" => ChunkType::Interface,
         "method" => ChunkType::Method,
         "typealias" => ChunkType::TypeAlias,
         "block" => ChunkType::Block,
         _ => ChunkType::Other,
      }
   }

   fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
      debug_assert_eq!(a.len(), b.len(), "cosine_similarity requires equal-length vectors");
      let len = a.len().min(b.len());
      let mut dot = 0.0;
      for i in 0..len {
         dot += a[i] * b[i];
      }
      dot
   }
}

impl Default for LanceStore {
   fn default() -> Self {
      Self::new().expect("failed to create LanceStore")
   }
}

impl LanceStore {
   pub async fn insert_segment_batch(
      &self,
      store_id: &str,
      table_name: &str,
      records: Vec<VectorRecord>,
   ) -> Result<()> {
      if records.is_empty() {
         return Ok(());
      }

      let table = self.get_table(store_id, table_name).await?;
      let batch = Self::records_to_batch(records)?;

      table
         .add(RecordBatchOnce::new(batch))
         .execute()
         .await
         .map_err(StoreError::AddRecords)?;

      Ok(())
   }

   pub async fn append_record_batch(
      &self,
      store_id: &str,
      table_name: &str,
      batch: RecordBatch,
   ) -> Result<()> {
      if batch.num_rows() == 0 {
         return Ok(());
      }
      let table = self.get_table(store_id, table_name).await?;
      table
         .add(RecordBatchOnce::new(batch))
         .execute()
         .await
         .map_err(StoreError::AddRecords)?;
      Ok(())
   }

   pub async fn list_tables(&self, store_id: &str) -> Result<Vec<String>> {
      let conn = self.get_connection(store_id).await?;
      conn
         .table_names()
         .execute()
         .await
         .map_err(StoreError::ListTables)
         .map_err(Into::into)
   }

   pub async fn drop_table(&self, store_id: &str, table_name: &str) -> Result<()> {
      let conn = self.get_connection(store_id).await?;
      conn
         .drop_table(table_name, &[])
         .await
         .map_err(StoreError::DropTable)?;
      Ok(())
   }

   pub async fn search_segments(&self, params: store::SearchParams<'_>) -> Result<SearchResponse> {
      if params.tables.is_empty() {
         return Ok(SearchResponse {
            results:    vec![],
            status:     SearchStatus::Ready,
            progress:   None,
            timings_ms: None,
            limits_hit: vec![],
            warnings:   vec![],
         });
      }

      let mut combined = SearchResponse {
         results:    Vec::new(),
         status:     SearchStatus::Ready,
         progress:   None,
         timings_ms: None,
         limits_hit: Vec::new(),
         warnings:   Vec::new(),
      };

      for table_name in params.tables {
         let table = match self.get_table(params.store_id, table_name).await {
            Ok(table) => table,
            Err(e) => {
               combined.warnings.push(crate::types::SearchWarning {
                  code:     "segment_open_failed".to_string(),
                  message:  format!("failed to open segment {table_name}: {e}"),
                  path_key: None,
               });
               continue;
            },
         };
         let response = self.search_table(&table, &params, table_name).await?;
         combined.results.extend(response.results);
         combined.limits_hit.extend(response.limits_hit);
         combined.warnings.extend(response.warnings);
      }

      Ok(combined)
   }

   async fn search_table(
      &self,
      table: &Table,
      params: &store::SearchParams<'_>,
      table_name: &str,
   ) -> Result<SearchResponse> {

      let anchor_filter = if params.include_anchors {
         "1 = 1"
      } else {
         "(kind IS NULL OR kind != 'anchor')"
      };
      let graph_clause = "(path_key LIKE '%.mmd' OR path_key LIKE '%.mermaid')";
      let doc_clause =
         "(path_key LIKE '%.md' OR path_key LIKE '%.mdx' OR path_key LIKE '%.txt' OR path_key LIKE \
          '%.json' OR path_key LIKE '%.yaml' OR path_key LIKE '%.yml' OR path_key LIKE '%.toml')";
      let non_code_clause = format!("({doc_clause} OR {graph_clause})");
      let code_clause = format!("NOT {non_code_clause}");

      let mut code_filter = format!("{code_clause} AND {anchor_filter}");
      let mut doc_filter = format!("{doc_clause} AND {anchor_filter}");
      let mut graph_filter = format!("{graph_clause} AND {anchor_filter}");
      let base_filter = if let Some(filter) = params.path_filter {
         let filter_str = store::escape_path_for_like(filter);
         let path_clause = format!("path_key LIKE '{filter_str}%'");
         code_filter = format!("{path_clause} AND {code_clause} AND {anchor_filter}");
         doc_filter = format!("{path_clause} AND {doc_clause} AND {anchor_filter}");
         graph_filter = format!("{path_clause} AND {graph_clause} AND {anchor_filter}");
         Some(format!("{path_clause} AND {anchor_filter}"))
      } else {
         Some(anchor_filter.to_owned())
      };

      let (code_batches, doc_batches, graph_batches): (
         Vec<RecordBatch>,
         Vec<RecordBatch>,
         Vec<RecordBatch>,
      ) = tokio::try_join!(
         async {
            let stream = table
               .query()
               .nearest_to(params.query_vector)
               .map_err(StoreError::CreateVectorQuery)?
               .limit(params.limit)
               .only_if(&code_filter)
               .execute()
               .await
               .map_err(StoreError::ExecuteCodeSearch)?;
            stream
               .try_collect()
               .await
               .map_err(StoreError::CollectCodeResults)
         },
         async {
            let stream = table
               .query()
               .nearest_to(params.query_vector)
               .map_err(StoreError::CreateVectorQuery)?
               .only_if(&doc_filter)
               .limit(params.limit)
               .execute()
               .await
               .map_err(StoreError::ExecuteDocSearch)?;
            stream
               .try_collect()
               .await
               .map_err(StoreError::CollectDocResults)
         },
         async {
            let stream = table
               .query()
               .nearest_to(params.query_vector)
               .map_err(StoreError::CreateVectorQuery)?
               .only_if(&graph_filter)
               .limit(params.limit)
               .execute()
               .await
               .map_err(StoreError::ExecuteDocSearch)?;
            stream
               .try_collect()
               .await
               .map_err(StoreError::CollectDocResults)
         },
      )?;

      let fts_query = FullTextSearchQuery::new(params.query_text.to_owned());
      let mut fts_query_builder = table.query().full_text_search(fts_query);

      if let Some(ref filter) = base_filter {
         fts_query_builder = fts_query_builder.only_if(filter);
      }

      let fts_batches: Vec<RecordBatch> =
         match fts_query_builder.limit(params.limit).execute().await {
            Ok(stream) => stream.try_collect().await.unwrap_or_default(),
            Err(_) => vec![],
         };

      let all_batches: Vec<&RecordBatch> = code_batches
         .iter()
         .chain(doc_batches.iter())
         .chain(graph_batches.iter())
         .chain(fts_batches.iter())
         .collect();

      let estimated_capacity = all_batches.iter().map(|b| b.num_rows()).sum();
      let mut candidates: Vec<(usize, usize)> = Vec::with_capacity(estimated_capacity);
      let mut seen_keys: HashSet<(&str, u32)> = HashSet::with_capacity(estimated_capacity);

      for (batch_idx, batch) in all_batches.iter().enumerate() {
         let path_col = batch
            .column_by_name("path_key")
            .ok_or(StoreError::MissingPathColumn)?
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or(StoreError::PathColumnTypeMismatch)?;

         let start_line_col = batch
            .column_by_name("start_line")
            .ok_or(StoreError::MissingStartLineColumn)?
            .as_any()
            .downcast_ref::<UInt32Array>()
            .ok_or(StoreError::StartLineTypeMismatch)?;

         for i in 0..batch.num_rows() {
            if path_col.is_null(i) {
               continue;
            }

            let path = path_col.value(i);
            let start_line = start_line_col.value(i);

            if !seen_keys.insert((path, start_line)) {
               continue;
            }

            candidates.push((batch_idx, i));
         }
      }

      let mut scored_results = Vec::with_capacity(candidates.len());

      for (cand_idx, (batch_idx, row_idx)) in candidates.iter().enumerate() {
         let batch = all_batches[*batch_idx];
         let row_id = batch
            .column_by_name("row_id")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(*row_idx)
            .to_string();
         let path: PathBuf = batch
            .column_by_name("path_key")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(*row_idx)
            .into();

         let content_col = batch.column_by_name("text").unwrap();
         let content = if let Some(str_array) = content_col.as_any().downcast_ref::<StringArray>() {
            str_array.value(*row_idx).to_string()
         } else if let Some(large_str_array) =
            content_col.as_any().downcast_ref::<LargeStringArray>()
         {
            large_str_array.value(*row_idx).to_string()
         } else {
            return Err(StoreError::ContentColumnTypeMismatch.into());
         };

         let start_line = batch
            .column_by_name("start_line")
            .unwrap()
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap()
            .value(*row_idx);

         let end_line = batch
            .column_by_name("end_line")
            .unwrap()
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap()
            .value(*row_idx);

         let chunk_type = batch.column_by_name("chunk_type").and_then(|col| {
            if col.is_null(*row_idx) {
               None
            } else {
               col.as_any()
                  .downcast_ref::<StringArray>()
                  .map(|arr| Self::parse_chunk_type(arr.value(*row_idx)))
            }
         });

         let is_anchor = batch.column_by_name("kind").and_then(|col| {
            if col.is_null(*row_idx) {
               None
            } else {
               col.as_any()
                  .downcast_ref::<StringArray>()
                  .map(|arr| arr.value(*row_idx) == "anchor")
            }
         });

         let vector_list = batch
            .column_by_name("embedding")
            .unwrap()
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .ok_or(StoreError::VectorColumnTypeMismatch)?;
         let vector_values = vector_list.value(*row_idx);
         let vector_floats = vector_values
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or(StoreError::VectorValuesTypeMismatch)?;

         let offset = vector_floats.offset();
         let len = vector_floats.len();
         let values = vector_floats.values();
         let doc_vector = &values[offset..offset + len];

         let score = Self::cosine_similarity(params.query_vector, doc_vector);

         let mut full_content = String::new();
         let mut context_prev_lines = 0u32;

         if let Some(prev_col) = batch.column_by_name("context_prev")
            && !prev_col.is_null(*row_idx)
            && let Some(prev_str) = prev_col.as_any().downcast_ref::<StringArray>()
         {
            let prev_content = prev_str.value(*row_idx);
            context_prev_lines = prev_content.lines().count() as u32;
            full_content.push_str(prev_content);
         }
         full_content.push_str(&content);
         if let Some(next_col) = batch.column_by_name("context_next")
            && !next_col.is_null(*row_idx)
            && let Some(next_str) = next_col.as_any().downcast_ref::<StringArray>()
         {
            full_content.push_str(next_str.value(*row_idx));
         }

         let adjusted_start_line = start_line.saturating_sub(context_prev_lines);

         scored_results.push((cand_idx, SearchResult {
            path,
            content: full_content.into(),
            score,
            secondary_score: None,
            row_id: Some(row_id),
            segment_table: Some(table_name.to_string()),
            start_line: adjusted_start_line,
            num_lines: end_line.saturating_sub(start_line).max(1),
            chunk_type,
            is_anchor,
         }));
      }

      scored_results.sort_by(|a, b| crate::types::cmp_results_deterministic(&a.1, &b.1));

      if params.rerank && !params.query_colbert.is_empty() {
         const RERANK_CAP: usize = 50;
         let rerank_count = scored_results.len().min(RERANK_CAP);

         for (cand_idx, result) in scored_results.iter_mut().take(rerank_count) {
            let (batch_idx, row_idx) = candidates[*cand_idx];
            let batch = all_batches[batch_idx];

            if let Some(colbert_col) = batch.column_by_name("colbert")
               && !colbert_col.is_null(row_idx)
            {
               let colbert_binary = if let Some(large_binary_array) =
                  colbert_col.as_any().downcast_ref::<LargeBinaryArray>()
               {
                  large_binary_array.value(row_idx)
               } else {
                  &[]
               };

               if !colbert_binary.is_empty() {
                  let scale = if let Some(scale_col) = batch.column_by_name("colbert_scale") {
                     if scale_col.is_null(row_idx) {
                        1.0
                     } else {
                        scale_col
                           .as_any()
                           .downcast_ref::<Float64Array>()
                           .map_or(1.0, |arr| arr.value(row_idx))
                     }
                  } else {
                     1.0
                  };

                  result.score = max_sim_quantized(
                     params.query_colbert,
                     colbert_binary,
                     scale,
                     config::get().colbert_dim,
                  );
               }
            }
         }

         scored_results.sort_by(|a, b| crate::types::cmp_results_deterministic(&a.1, &b.1));
      }

      let mut scored_results: Vec<SearchResult> =
         scored_results.into_iter().map(|(_, r)| r).collect();
      scored_results.truncate(params.limit);

      Ok(SearchResponse {
         results:    scored_results,
         status:     SearchStatus::Ready,
         progress:   None,
         timings_ms: None,
         limits_hit: vec![],
         warnings:   vec![],
      })
   }

   pub async fn delete_store(&self, store_id: &str) -> Result<()> {
      self.connections.write().remove(store_id);
      let path = self.data_dir.join(store_id);
      if path.exists() {
         fs::remove_dir_all(&path)?;
      }
      Ok(())
   }

   pub async fn is_empty(&self, store_id: &str) -> Result<bool> {
      let tables = self.list_tables(store_id).await.unwrap_or_default();
      Ok(tables.is_empty())
   }

   pub async fn create_fts_index(&self, store_id: &str, table_name: &str) -> Result<()> {
      let table = self.get_table(store_id, table_name).await?;

      table
         .create_index(&["text"], Index::FTS(Default::default()))
         .execute()
         .await
         .map_err(|e| {
            if matches!(e, lancedb::Error::TableAlreadyExists { .. }) {
               return StoreError::IndexAlreadyExists;
            }
            StoreError::CreateFtsIndex(e)
         })?;

      Ok(())
   }

   pub async fn create_vector_index(&self, store_id: &str, table_name: &str) -> Result<()> {
      let table = self.get_table(store_id, table_name).await?;

      let vector_rows = table
         .count_rows(Some("embedding IS NOT NULL".to_string()))
         .await
         .map_err(StoreError::CountRows)?;

      if vector_rows < 1000 {
         return Ok(());
      }

      let mut num_partitions = (vector_rows / 100).clamp(8, 64) as u32;
      num_partitions = num_partitions.min(vector_rows as u32).max(1);

      let index = Index::IvfPq(
         lancedb::index::vector::IvfPqIndexBuilder::default().num_partitions(num_partitions),
      );

      if let Err(e) = table.create_index(&["embedding"], index).execute().await {
         tracing::warn!("skipping vector index for {table_name} (rows={vector_rows}): {e}");
         return Ok(());
      }

      Ok(())
   }

   pub async fn segment_metadata(
      &self,
      store_id: &str,
      table_name: &str,
   ) -> Result<store::SegmentMetadata> {
      let table = self.get_table(store_id, table_name).await?;
      let row_count = table
         .count_rows(None)
         .await
         .map_err(StoreError::CountRows)? as u64;

      let dataset_uri = table.dataset_uri();
      let dataset_path = dataset_uri.strip_prefix("file://").unwrap_or(dataset_uri);
      let (size_bytes, sha256) = crate::snapshot::compute_dir_hash(Path::new(dataset_path))?;

      Ok(store::SegmentMetadata { rows: row_count, size_bytes, sha256 })
   }

   pub fn store_path(&self, store_id: &str) -> PathBuf {
      self.data_dir.join(store_id)
   }
}
