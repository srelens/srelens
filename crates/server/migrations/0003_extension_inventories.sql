-- One app inventory per web user (#515): the same JSON document the desktop keeps in
-- its extensions file, checked the same way on every read. Written only through that
-- user's `extensions.configure`, never through /api/settings, so a user cannot place
-- an inventory the install checks never saw.
CREATE TABLE extension_inventories (
    user_id    INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    inventory  BLOB NOT NULL,
    updated_at INTEGER NOT NULL
);
