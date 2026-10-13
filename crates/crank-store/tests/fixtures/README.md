# Released-schema migration fixtures

No schema has been released yet. `v1.db` is the schema-v1 baseline (SQLite
user_version 1), with a versioned setting proving data survives opening.
`migrate_every_released_fixture` copies each immutable fixture and opens it through
the real owner path. Every future forward migration must add its prior released
schema fixture to that test, update its expected current version, and assert
preservation of representative data. Never regenerate an older fixture from the
current migrations. Empty→current is tested separately.
