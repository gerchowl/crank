"""Build crates/crank-store/tests/fixtures/record-v1.db: schema v5 (migrations 0001-0005), one
finished ride with record_version 1 and a version-1 record (power, cadence, speed, hr,
workout_s, block), 75 seconds in two chunks, in the ride_chunk format of ADR-0001 §6
(n little-endian f32, NaN = missing; estimated bitmask, LSB first)."""
import math, sqlite3, struct, sys, os
root, out = sys.argv[1], sys.argv[2]
db = sqlite3.connect(out)
for i in range(1, 6):
    name = sorted(f for f in os.listdir(f"{root}/crates/crank-store/migrations") if f.startswith(f"000{i}_"))[0]
    db.executescript(open(f"{root}/crates/crank-store/migrations/{name}").read())
db.execute("PRAGMA user_version = 5")
NAN = float("nan")
def row(s):
    paused = 50 <= s < 55
    return {
        "power": NAN if paused or s % 13 == 0 else 100.0 + s,
        "cadence": NAN if paused else 90.0,
        "speed": NAN if paused else 8.5,
        "hr": NAN if paused or 40 <= s < 45 else 120.0 + s % 10,
        "workout_s": NAN if paused else float(min(s, 50) + 1 if s < 50 else s - 4),
        "block": NAN if paused else (0.0 if s < 30 else 1.0),
    }
wj = '{"v":1,"id":"w","name":"Ride","ftp":250,"nodes":[]}'
db.execute("INSERT INTO ride(id,started_at,ended_at,state,workout_json,compiled_json,telemetry_source,kind,summary_json)"
           " VALUES ('v1-ride',1700000000,1700000075,'finished',?,'{\"v\":1}','sim','workout','{\"v\":1}')", (wj,))
for metric in ["power", "cadence", "speed", "hr", "workout_s", "block"]:
    for start in (0, 60):
        secs = range(start, min(start + 60, 75))
        data = b"".join(struct.pack("<f", row(s)[metric]) for s in secs)
        mask = bytes((len(secs) + 7) // 8)
        db.execute("INSERT INTO ride_chunk VALUES (?,?,?,?,?,?)", ("v1-ride", metric, start, len(secs), data, mask))
db.commit()
db.execute("VACUUM")
db.close()
