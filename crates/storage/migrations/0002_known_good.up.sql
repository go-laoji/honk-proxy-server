ALTER TABLE config_revisions ADD COLUMN applied_at INTEGER;
UPDATE config_revisions SET applied_at=created_at WHERE id IN (
    SELECT revision FROM config_activations WHERE state='applied'
    UNION SELECT last_applied_revision FROM config_head WHERE last_applied_revision IS NOT NULL
);
