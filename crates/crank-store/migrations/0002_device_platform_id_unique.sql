-- #147 / #166: one platform peripheral id is one remembered device (ADR-0002 §5).
-- Collapse any duplicates onto the most recently seen row, repointing rides first.
CREATE TEMP TABLE device_keep AS
SELECT d.id AS old_id,
       (SELECT k.id FROM device k WHERE k.platform_id = d.platform_id
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
CREATE UNIQUE INDEX device_platform_id ON device(platform_id);
