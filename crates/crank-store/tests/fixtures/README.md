# Released-schema migration fixtures

No schema has been released yet. `v1.db` is the schema-v1 baseline (SQLite
user_version 1), with a versioned setting proving data survives opening. `v2.db` is the
schema-v2 baseline (user_version 2, migrations 0001+0002) with an `athlete_profile` row whose
`par_q_result` migration 0003 renames (#210). `v3.db` is the schema-v3 baseline (user_version 3, migrations
0001-0003, built with Python's sqlite3 from those files) with the setting and one finished workout ride,
which migration 0004 (#541, routes) must keep with no route. `v4.db` is the schema-v4 baseline (user_version 4,
migrations 0001-0004, built with Python's sqlite3 from those files) with the setting, three finished rides (a workout
`workout-ride`, a free ride `free-ride-1` with workout id `free-ride`, and a route ride `route-ride` with `route_json`),
an `athlete_profile` row (weight 72.5 kg, age 40), two `athlete_estimate` rows (FTP 260 W, HR max 185 bpm) and a
check-in, which migration 0005 (#714, ADR-0009 §2.5) must carry over: estimates kept as method `legacy`, the weight
moved into `athlete_estimate`, the ride kinds back-filled.
`migrate_every_released_fixture` copies each immutable fixture and opens it through
the real owner path. Every future forward migration must add its prior released
schema fixture to that test, update its expected current version, and assert
preservation of representative data. Never regenerate an older fixture from the
current migrations. Empty→current is tested separately.
