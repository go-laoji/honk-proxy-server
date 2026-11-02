CREATE TABLE config_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id INTEGER REFERENCES config_revisions(id) ON DELETE RESTRICT,
    document TEXT NOT NULL CHECK(json_valid(document) AND length(document) BETWEEN 1 AND 2097152),
    checksum TEXT NOT NULL CHECK(length(checksum) = 64),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    note TEXT NOT NULL CHECK(length(note) <= 256)
);
CREATE INDEX config_revisions_parent ON config_revisions(parent_id);
CREATE TABLE config_head (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    desired_revision INTEGER NOT NULL REFERENCES config_revisions(id) ON DELETE RESTRICT,
    last_applied_revision INTEGER REFERENCES config_revisions(id) ON DELETE RESTRICT
);
CREATE INDEX config_head_applied ON config_head(last_applied_revision);
CREATE INDEX config_head_desired ON config_head(desired_revision);
CREATE TABLE config_draft (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    version INTEGER NOT NULL CHECK(version >= 1),
    base_revision INTEGER NOT NULL REFERENCES config_revisions(id) ON DELETE RESTRICT,
    document TEXT NOT NULL CHECK(json_valid(document) AND length(document) BETWEEN 1 AND 2097152),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0)
);
CREATE INDEX config_draft_base ON config_draft(base_revision);
CREATE TABLE config_activations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    revision INTEGER NOT NULL REFERENCES config_revisions(id) ON DELETE RESTRICT,
    previous_revision INTEGER NOT NULL REFERENCES config_revisions(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK(state IN ('prepared','applied','pending_restart','failed','interrupted')),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    error_code TEXT
);
CREATE INDEX config_activations_revision ON config_activations(revision, id DESC);
CREATE INDEX config_activations_previous ON config_activations(previous_revision);
CREATE INDEX config_activations_state ON config_activations(state, id DESC);
