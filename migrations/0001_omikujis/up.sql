START TRANSACTION;

CREATE TABLE `omikujis` (
  `id` int(10) UNSIGNED NOT NULL,
  `photo` varchar(32) NULL COMMENT 'file_id of the photo received',
  `message` varchar(32) NOT NULL COMMENT 'in a serialized format',
  `vote_count` int(10) NOT NULL DEFAULT 0 COMMENT '+/-, a message will be hidden if it is <=-3.',
  `tg_id` bigint(10) NOT NULL,
  `tg_name` varchar(32) NOT NULL,
  `created_at` timestamp NOT NULL DEFAULT current_timestamp(),
  `updated_at` timestamp NOT NULL DEFAULT current_timestamp() ON UPDATE current_timestamp()
) DEFAULT CHARSET=utf8mb4;

-- Add auto increment and primary key
ALTER TABLE `omikujis`
  ADD PRIMARY KEY (`id`);

ALTER TABLE `omikujis`
  MODIFY `id` int(10) UNSIGNED NOT NULL AUTO_INCREMENT;

COMMIT;