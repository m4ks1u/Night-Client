-- 0004: optional custom game directory per instance (NULL = <data>/instances/<dir_name>).
ALTER TABLE instances ADD COLUMN game_dir TEXT CHECK (game_dir IS NULL OR length(game_dir) BETWEEN 3 AND 1024);
