CREATE TABLE file_sources(delivery_id TEXT NOT NULL REFERENCES deliveries(id),file_id TEXT NOT NULL,source_json TEXT NOT NULL,PRIMARY KEY(delivery_id,file_id));
CREATE TABLE receive_outputs(delivery_id TEXT NOT NULL REFERENCES deliveries(id),file_id TEXT NOT NULL,source_json TEXT NOT NULL,published INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(delivery_id,file_id));
