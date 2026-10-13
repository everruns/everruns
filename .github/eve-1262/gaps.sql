-- Mean and p95 gap (ms) between consecutive events of a turn, by transition.
WITH e AS (
  SELECT session_id, sequence, event_type, ts,
         lag(event_type) OVER w AS prev_type, lag(ts) OVER w AS prev_ts
  FROM events
  WINDOW w AS (PARTITION BY session_id ORDER BY sequence)
)
SELECT prev_type || ' -> ' || event_type AS step, count(*) AS n,
       round(avg(extract(epoch FROM ts - prev_ts) * 1000)::numeric, 1) AS avg_ms,
       round((percentile_cont(0.95) WITHIN GROUP (ORDER BY extract(epoch FROM ts - prev_ts) * 1000))::numeric, 1) AS p95_ms,
       round(max(extract(epoch FROM ts - prev_ts) * 1000)::numeric, 1) AS max_ms
FROM e
WHERE prev_type IS NOT NULL AND prev_type <> 'turn.completed' AND event_type <> 'input.message'
GROUP BY 1
HAVING count(*) > 20
ORDER BY avg_ms DESC
LIMIT 25;
