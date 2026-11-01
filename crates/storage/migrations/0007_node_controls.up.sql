CREATE TABLE node_controls (
    node_id TEXT PRIMARY KEY CHECK(length(node_id) BETWEEN 1 AND 70),
    state TEXT NOT NULL CHECK(state IN ('enabled','disabled','draining','deleted')),
    version INTEGER NOT NULL CHECK(version >= 1),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0)
);
