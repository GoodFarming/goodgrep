use ggrep::util::sanitize_output;

#[test]
fn sanitize_control_chars() {
   let input = "ok\x1b[31mred\x1b[0m\x07\tline\nnext";
   let sanitized = sanitize_output(input);
   assert!(!sanitized.contains('\u{1b}'));
   assert!(!sanitized.contains('\u{7}'));
   assert!(sanitized.contains("okred"));
   assert!(sanitized.contains("\tline\nnext"));
}
