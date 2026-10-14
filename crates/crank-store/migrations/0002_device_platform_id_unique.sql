-- #147 / #166: a device row is one (peripheral, role) pairing (ADR-0002 §5): the same
-- platform peripheral may be both trainer and strap, but each role appears once.
-- Collapse duplicates *within a kind* onto the most recently seen row, repointing rides
-- first; rows of different kinds are never merged. Also add the DIS model (2A24).
ALTER TABLE device ADD COLUMN model TEXT;
CREATE TEMP TABLE device_keep AS
SELECT d.id AS old_id,
       (SELECT k.id FROM device k WHERE k.platform_id = d.platform_id AND k.kind = d.kind
        ORDER BY k.last_seen DESC, k.id DESC LIMIT 1) AS new_id
FROM device d;
UPDATE ride SET trainer_device_id =
    (SELECT new_id FROM device_keep WHERE old_id = trainer_device_id)
 WHERE trainer_device_id IS NOT NULL;
UPDATE ride SET hr_device_id =
    (SELECT new_id FROM device_keep WHERE old_id = hr_device_id)
 WHERE hr_device_id IS NOT NULL;
DELETE FROM device WHERE id IN (SELECT old_id FROM device_keep WHERE old_id <> new_id);
DROP TABLE device_keep;
CREATE UNIQUE INDEX device_platform_kind ON device(platform_id, kind);
