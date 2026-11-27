ALTER TABLE saved_files RENAME TO previous_saved_files;
CREATE TABLE saved_files(
    message_id TEXT NOT NULL,
    file_id TEXT NOT NULL,
    transfer_id TEXT NOT NULL REFERENCES deliveries(id),
    name TEXT NOT NULL,
    directory TEXT NOT NULL DEFAULT '',
    location TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL CHECK(status IN ('pending','saved','failed','replaced')),
    error TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(message_id,file_id),
    FOREIGN KEY(message_id,file_id) REFERENCES attachments(message_id,file_id)
);
INSERT INTO saved_files SELECT * FROM previous_saved_files;
DROP TABLE previous_saved_files;
CREATE INDEX saved_file_pending ON saved_files(status,updated_at);
