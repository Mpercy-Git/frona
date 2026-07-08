use frona_derive::migration;

#[migration("2026-06-16T00:00:00Z")]
fn drop_refuse_user_delete_event() -> &'static str {
    "REMOVE EVENT IF EXISTS refuse_user_delete_with_owned ON TABLE user;"
}
