# Released-schema migration fixtures

No schema has been released yet. `v1.db` is the schema-v1 baseline (SQLite
user_version 1), with a versioned setting proving data survives opening. `v2.db` is the
schema-v2 baseline (user_version 2, migrations 0001+0002) with an `athlete_profile` row whose
`par_q_result` migration 0003 renames (#210).
`migrate_every_released_fixture` copies each immutable fixture and opens it through
the real owner path. Every future forward migration must add its prior released
schema fixture to that test, update its expected current version, and assert
preservation of representative data. Never regenerate an older fixture from the
current migrations. Empty→current is tested separately.
