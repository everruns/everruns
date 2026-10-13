-- c1 server time by turn number within a session.
WITH c1 AS (
  SELECT session_id FROM events GROUP BY session_id ORDER BY min(ts) LIMIT 21
), e AS (
  SELECT e.session_id, e.event_type, e.ts,
         sum(CASE WHEN e.event_type = 'input.message' THEN 1 ELSE 0 END)
           OVER (PARTITION BY e.session_id ORDER BY e.sequence) AS turn_no
  FROM events e JOIN c1 USING (session_id)
), turns AS (
  SELECT session_id, turn_no,
         extract(epoch FROM max(ts) FILTER (WHERE event_type = 'turn.completed')
                          - min(ts) FILTER (WHERE event_type = 'input.message')) * 1000 AS server_ms
  FROM e GROUP BY 1, 2
)
SELECT turn_no, count(*), round(avg(server_ms)::numeric, 1) AS avg_ms,
       round(max(server_ms)::numeric, 1) AS max_ms
FROM turns WHERE turn_no > 0 GROUP BY 1 ORDER BY 1;
