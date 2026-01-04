use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn test_ipc_rejects_truncated_length_prefix() {
   let (mut client, mut server) = tokio::io::duplex(16);
   client.write_all(&[0x01, 0x02]).await.unwrap();
   drop(client);

   let mut buffer = ggrep::ipc::SocketBuffer::new();
   let err = buffer
      .recv_with_limit::<_, ggrep::ipc::Request>(&mut server, 1024)
      .await
      .unwrap_err();
   assert!(err.to_string().contains("failed to read"));
}

#[tokio::test]
async fn test_ipc_rejects_truncated_payload() {
   let (mut client, mut server) = tokio::io::duplex(32);
   let len: u32 = 10;
   client.write_all(&len.to_le_bytes()).await.unwrap();
   client.write_all(&[0xaa; 5]).await.unwrap();
   drop(client);

   let mut buffer = ggrep::ipc::SocketBuffer::new();
   let err = buffer
      .recv_with_limit::<_, ggrep::ipc::Request>(&mut server, 1024)
      .await
      .unwrap_err();
   assert!(err.to_string().contains("failed to read"));
}

#[tokio::test]
async fn test_ipc_rejects_garbage_payload() {
   let (mut client, mut server) = tokio::io::duplex(64);
   let payload = [0xffu8; 16];
   let len = payload.len() as u32;
   client.write_all(&len.to_le_bytes()).await.unwrap();
   client.write_all(&payload).await.unwrap();
   drop(client);

   let mut buffer = ggrep::ipc::SocketBuffer::new();
   let err = buffer
      .recv_with_limit::<_, ggrep::ipc::Request>(&mut server, 1024)
      .await
      .unwrap_err();
   assert!(err.to_string().contains("failed to deserialize"));
}

#[tokio::test]
async fn test_ipc_rejects_oversized_payload() {
   let (mut client, mut server) = tokio::io::duplex(32);
   let len: u32 = 2048;
   client.write_all(&len.to_le_bytes()).await.unwrap();
   drop(client);

   let mut buffer = ggrep::ipc::SocketBuffer::new();
   let err = buffer
      .recv_with_limit::<_, ggrep::ipc::Request>(&mut server, 16)
      .await
      .unwrap_err();
   assert!(err.to_string().contains("message too large"));
}
