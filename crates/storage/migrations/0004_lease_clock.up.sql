ALTER TABLE provider_clock ADD COLUMN clean_shutdown INTEGER NOT NULL DEFAULT 0 CHECK(clean_shutdown IN (0,1));
