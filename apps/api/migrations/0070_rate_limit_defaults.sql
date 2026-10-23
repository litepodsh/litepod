-- The global API quota is keyed per client IP, and the organization lists used
-- to fan out one request per application: 35+ calls for a 31-app organization,
-- past a burst of 40 on a single page load, so some rows came back 429 and
-- rendered empty. The lists now make one request, but the burst was also tight
-- for ordinary dashboard traffic, so a fresh install starts at 40 requests per
-- second with a burst of 80. Only installations still on the untouched
-- defaults move: anything an operator tuned stays exactly as they set it.
UPDATE rate_limit_settings
   SET global_per_second = 40, global_burst = 80
 WHERE id = 1 AND global_per_second = 20 AND global_burst = 40;
