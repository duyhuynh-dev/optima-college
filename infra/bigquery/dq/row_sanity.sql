-- Post-load sanity (run in BigQuery console or bq query). Replace project/dataset/term.
-- Checkpoint A: optional checks after bq_load.

-- Row counts for a term (should match ingest expectations within drift policy)
SELECT 'courses' AS tbl, COUNT(*) AS n
FROM `optima-college.optima.courses`
WHERE term = '1269'
UNION ALL
SELECT 'sections', COUNT(*)
FROM `optima-college.optima.sections`
WHERE term = '1269'
UNION ALL
SELECT 'meetings', COUNT(*)
FROM `optima-college.optima.meetings`
WHERE term = '1269';

-- Required fields should not be null
SELECT COUNT(*) AS bad_courses
FROM `optima-college.optima.courses`
WHERE term = '1269'
  AND (term IS NULL OR subject_code IS NULL OR course_code IS NULL OR course_title IS NULL);
