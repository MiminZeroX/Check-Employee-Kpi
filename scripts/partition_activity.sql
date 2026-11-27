-- One-time conversion of activity_events (the largest table) to monthly partitions.
--
-- Why: with many cameras activity_events grows by millions of rows a year. Monthly
-- partitions make date-range reports read only the months they need, and data older
-- than the retention period is removed by dropping a whole partition (instant)
-- instead of deleting rows one by one.
--
-- Run once, during a quiet time (the table is rebuilt; ~1 min per few million rows):
--   docker compose exec -T mysql mariadb -usmartoffice -p"$MYSQL_PASSWORD" smartoffice < ../scripts/partition_activity.sql
-- or with a local client:
--   mariadb -usmartoffice -p smartoffice < scripts/partition_activity.sql
--
-- After that the server maintains the partitions by itself (adds the next months,
-- drops months older than "retention_days"). Running the script again does nothing.
-- Changes made: the foreign key data_corrections -> activity_events is replaced by
-- application-level cleanup (partitioned tables cannot have foreign keys), and the
-- primary key becomes (id, started_at).

DELIMITER //
DROP PROCEDURE IF EXISTS so_partition_activity//
CREATE PROCEDURE so_partition_activity(IN months_ahead INT)
BEGIN
  DECLARE m DATE;
  DECLARE stop_at DATE;
  DECLARE parts TEXT DEFAULT '';
  IF EXISTS (SELECT 1 FROM information_schema.PARTITIONS
              WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'activity_events' AND PARTITION_NAME IS NOT NULL) THEN
    SELECT 'activity_events is already partitioned - nothing to do' AS result;
  ELSE
    IF EXISTS (SELECT 1 FROM information_schema.TABLE_CONSTRAINTS
                WHERE CONSTRAINT_SCHEMA = DATABASE() AND TABLE_NAME = 'data_corrections'
                  AND CONSTRAINT_NAME = 'fk_corr_event' AND CONSTRAINT_TYPE = 'FOREIGN KEY') THEN
      ALTER TABLE data_corrections DROP FOREIGN KEY fk_corr_event;
    END IF;
    -- one partition per month from the oldest data to `months_ahead` months from now
    SELECT DATE_FORMAT(COALESCE(MIN(started_at), UTC_TIMESTAMP()), '%Y-%m-01') + INTERVAL 1 MONTH INTO m FROM activity_events;
    SET stop_at = DATE_FORMAT(UTC_TIMESTAMP(), '%Y-%m-01') + INTERVAL (months_ahead + 1) MONTH;
    WHILE m <= stop_at DO
      SET parts = CONCAT(parts, 'PARTITION p', DATE_FORMAT(m - INTERVAL 1 MONTH, '%Y%m'),
                         ' VALUES LESS THAN (''', DATE_FORMAT(m, '%Y-%m-%d'), '''), ');
      SET m = m + INTERVAL 1 MONTH;
    END WHILE;
    SET @so_sql = CONCAT('ALTER TABLE activity_events DROP PRIMARY KEY, ADD PRIMARY KEY (id, started_at) ',
                         'PARTITION BY RANGE COLUMNS(started_at) (', parts, 'PARTITION pmax VALUES LESS THAN (MAXVALUE))');
    PREPARE so_stmt FROM @so_sql;
    EXECUTE so_stmt;
    DEALLOCATE PREPARE so_stmt;
    SELECT CONCAT('activity_events partitioned: ', COUNT(*), ' partitions') AS result
      FROM information_schema.PARTITIONS
     WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'activity_events';
  END IF;
END//
DELIMITER ;

CALL so_partition_activity(3);
DROP PROCEDURE so_partition_activity;
