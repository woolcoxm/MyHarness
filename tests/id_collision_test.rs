//! Two myharness processes sharing one store must not erase each other's
//! captures (found by inspection: both counted ids from 1 and the
//! cross-process merge dedupes by id).
use myharness::zero_mem::{ZeroMem, ZeroMemCfg};
use std::collections::HashSet;
use std::path::Path;

#[tokio::test]
async fn concurrent_stores_do_not_lose_units() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ZeroMemCfg::default();
    let mut a = ZeroMem::open(dir.path(), Path::new("/w/p"), "sess-A-111111", cfg.clone());
    let mut b = ZeroMem::open(dir.path(), Path::new("/w/p"), "sess-B-222222", cfg);
    a.capture("user", "fact one from process A");
    b.capture("user", "fact two from process B");
    a.persist();
    b.persist(); // merge must keep A's units AND B's
    let c = ZeroMem::open(dir.path(), Path::new("/w/p"), "sess-C", ZeroMemCfg::default());
    assert_eq!(c.count(), 2, "units lost to the id collision");
    let hits_b = c.retrieve("fact two from process", &HashSet::new());
    assert!(
        hits_b.iter().any(|h| h.snippet.contains("process B")),
        "foreign unit missing: {hits_b:?}"
    );
}
