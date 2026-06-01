// SurrealValue derive can't default a missing object to {} on read.

use frona_derive::migration;

#[migration("2026-05-07T00:00:00Z")]
fn backfill_empty_metadata() -> &'static str {
    "UPDATE chat    SET metadata = {} WHERE metadata IS NONE;
     UPDATE message SET metadata = {} WHERE metadata IS NONE;
     UPDATE contact SET metadata = {} WHERE metadata IS NONE;
     UPDATE space   SET metadata = {} WHERE metadata IS NONE;"
}
