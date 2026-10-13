-- The slowest c1 turns (first 21 sessions), with each step's gap in ms.
WITH c1 AS (
  SELECT session_id FROM events GROUP BY session_id ORDER BY min(ts) LIMIT 21
), e AS (
  SELECT e.session_id, e.sequence, e.event_type, e.ts,
         lag(e.event_type) OVER w AS prev_type, lag(e.ts) OVER w AS prev_ts,
         sum(CASE WHEN e.event_type = 'input.message' THEN 1 ELSE 0 END) OVER w AS turn_no
  FROM events e JOIN c1 USING (session_id)
  WINDOW w AS (PARTITION BY e.session_id ORDER BY e.sequence)
), turns AS (
  SELECT session_id, turn_no,
         extract(epoch FROM max(ts) FILTER (WHERE event_type = 'turn.completed')
                          - min(ts) FILTER (WHERE event_type = 'input.message')) * 1000 AS server_ms
  FROM e GROUP BY 1, 2
), worst AS (
  SELECT * FROM turns ORDER BY server_ms DESC NULLS LAST LIMIT 12
)
SELECT w.session_id, w.turn_no, round(w.server_ms::numeric, 1) AS server_ms,
       string_agg(e.prev_type || '>' || e.event_type || '=' || round((extract(epoch FROM e.ts - e.prev_ts) * 1000)::numeric, 0), '  ' ORDER BY e.sequence)
         FILTER (WHERE extract(epoch FROM e.ts - e.prev_ts) * 1000 > 3) AS slow_steps
FROM worst w JOIN e ON e.session_id = w.session_id AND e.turn_no = w.turn_no AND e.prev_type IS NOT NULL
GROUP BY 1, 2, 3 ORDER BY 3 DESC;
