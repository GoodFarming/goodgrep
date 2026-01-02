use std::{path::PathBuf, sync::Arc};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<S>(arc_path: &Arc<PathBuf>, serializer: S) -> Result<S::Ok, S::Error>
where
   S: Serializer,
{
   arc_path.as_ref().serialize(serializer)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<PathBuf>, D::Error>
where
   D: Deserializer<'de>,
{
   PathBuf::deserialize(deserializer).map(Arc::new)
}
