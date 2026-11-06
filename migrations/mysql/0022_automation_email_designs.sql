CREATE TABLE email_design_settings (
    `id` VARCHAR(191) PRIMARY KEY,
    `settings` VARCHAR(4000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
