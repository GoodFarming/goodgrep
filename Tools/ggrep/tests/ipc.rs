use ggrep::ipc;

#[test]
fn handshake_highest_common_version() {
   let negotiated = ipc::negotiate_protocol(&[1, 3, 2, 99]);
   assert_eq!(negotiated, Some(2));
}
