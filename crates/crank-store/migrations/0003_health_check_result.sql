-- #210: crank's screen is "informed by" the PAR-Q+, not the PAR-Q+ itself (#193), so the column
-- must not carry that name. A rename keeps every stored value.
ALTER TABLE athlete_profile RENAME COLUMN par_q_result TO health_check_result;
