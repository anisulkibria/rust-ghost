-- The live association is separate from historical tokens, as in Ghost 6.73.
-- Legacy hashed links remain valid until their post's next reset/revocation.
CREATE TABLE gift_links (
    `token` VARCHAR(191) NOT NULL PRIMARY KEY,
    `post_id` VARCHAR(191) NOT NULL ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX gift_links_post_id ON gift_links (`post_id`);
CREATE TABLE post_gift_links (
    `post_id` VARCHAR(191) NOT NULL UNIQUE ,
    `gift_link_token` VARCHAR(191) NOT NULL PRIMARY KEY ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE gift_links ADD CONSTRAINT gift_links_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE post_gift_links ADD CONSTRAINT post_gift_links_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE post_gift_links ADD CONSTRAINT post_gift_links_gift_link_token_fk2 FOREIGN KEY (`gift_link_token`) REFERENCES gift_links(`token`) ON DELETE CASCADE;
