-- 0009: the id of the last modpack update recorded for an instance. It is
-- saved in the same transaction as the pack, its files and the instance's
-- Minecraft/loader versions, and acknowledges exactly that update: after a
-- crash, the launcher keeps an update's files only when this id matches the
-- update's journal (otherwise it undoes them). NULL after an import.

ALTER TABLE instance_modpack
    ADD COLUMN last_update_id TEXT CHECK (last_update_id IS NULL OR length(last_update_id) BETWEEN 1 AND 64);
