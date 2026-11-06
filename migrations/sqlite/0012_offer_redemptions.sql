-- Which offer a member signed up through.
--
-- Ghost keeps redemptions in their own table so one member can redeem several
-- offers over time. A member here can only arrive through the one offer that
-- was on their first paid checkout, which is what the Offers screen counts, so
-- the link lives on the member row.
ALTER TABLE members ADD COLUMN offer_id TEXT(24) REFERENCES offers(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS members_offer_id_index ON members (offer_id);
