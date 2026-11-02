-- Offline rollback for an empty/disposable database only. Production rollback
-- restores validated data; the running daemon never executes destructive DOWN.
DROP TABLE config_activations;
DROP TABLE config_draft;
DROP TABLE config_head;
DROP TABLE config_revisions;
