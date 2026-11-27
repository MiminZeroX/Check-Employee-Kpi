-- Phase 6: MFA replay protection and SSO
ALTER TABLE users
  ADD COLUMN totp_last_step BIGINT NULL AFTER totp_secret,
  ADD COLUMN email VARCHAR(200) NULL AFTER display_name;
