//! Runtime assertion stubs for conformance mapping.

/// Lease epoch preflight enforcement (IDX.MUST.002).
#[allow(dead_code)]
pub fn lease_epoch_preflight(
   store_id: &str,
   owner_id: &str,
   lease_epoch: u64,
) -> crate::Result<()> {
   crate::lease::verify_lease_owner(store_id, owner_id, lease_epoch)
}

/// Placeholder for IDX.MUST.004 GC requires writer lease enforcement.
#[allow(dead_code)]
pub fn gc_requires_writer_lease(
   store_id: &str,
   owner_id: &str,
   lease_epoch: u64,
) -> crate::Result<()> {
   crate::lease::verify_lease_owner(store_id, owner_id, lease_epoch)
}
