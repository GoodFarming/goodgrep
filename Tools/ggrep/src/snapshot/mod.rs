//! Snapshot manifest types and utilities.

pub mod manifest;
pub mod manager;
pub(crate) mod pins;
pub mod segment_index;
pub mod view;
pub mod compaction;
pub mod gc;

pub use manifest::{
   SnapshotCounts, SnapshotError, SnapshotGitInfo, SnapshotManifest, SnapshotSegmentRef,
   SnapshotTombstoneRef,
};
pub use manager::{SnapshotManager, compute_dir_hash, compute_tombstone_artifact, segment_table_name};
pub use segment_index::{SegmentFileIndexEntry, read_segment_file_index, write_segment_file_index};
pub use view::SnapshotView;
pub use compaction::{CompactionOptions, CompactionResult, compact_store, compaction_overdue};
pub use gc::{GcOptions, GcReport, gc_snapshots};
