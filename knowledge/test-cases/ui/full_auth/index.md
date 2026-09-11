# Full auth (UI)

* [TC001: Full Auth - User Sign-up](TC001_user_signup.md) - Verify that a new user can sign up through the explicit \"Create an account\" path when AUTH_MODE is full and signup is enabled.
* [TC002: Full Auth - Sign-out After Sign-in](TC002_signout_after_signin.md) - Verify that a user can successfully sign out after signing in when AUTH_MODE is set to full.
* [TC003: Full Auth - Login with Signed Up User, Sign-out](TC003_login_signout_flow.md) - Verify the complete flow of logging in with a previously signed up user and then signing out.
* [TC004: Full Auth - Failed Login (Wrong Password / Unknown User)](TC004_failed_login_random_user.md) - Verify that authentication fails with a calm generic message when the password is wrong or the account is unknown, and that the login door never reveals whether an account exists for an email.
* [TC005: Full Auth - Password Reset Flow](TC005_password_reset_flow.md) - Verify the full self-service password reset: request link, set a new password, old sessions revoked, expired/invalid links handled gracefully.
* [TC006: Full Auth - Email Verification Flow](TC006_email_verification_flow.md) - Verify email verification: pending state, resend with cooldown, wrong-address recovery, token consumption, invalid/expired token handling.
* [TC007: Full Auth - OAuth Sign-in Error Paths](TC007_oauth_error_paths.md) - Verify OAuth (Google/GitHub) failure handling: the callback never dead-ends on raw JSON; the login door shows friendly copy per category.
* [TC008: Full Auth - Reachability Dead-End & Trap Recovery](TC008_reachability_dead_end_recovery.md) - Walk the scenarios the reachability model (`apps/ui/src/lib/auth-flow/machine.ts`) exists to protect: situations where a screen historically offered a way forward that silently no-ops or dead-ends for a particular hid...
* [TC009 — Invited user signs up and accepts invitation](TC009_invited_user_signup_resume.md) - Verifies that an organization invite link preserves its target through the login → signup → email verification path and resumes invitation acceptance for a brand-new user.
* [TC010: Full Auth - External Login Origin](TC010_external_login_origin.md) - Verify that protected routes can delegate the login page to a trusted remote origin while preserving a safe relative `return_to` continuation.
* [Full Auth — State-Machine Coverage Map](coverage-map.md) - Maps each auth-flow state-machine scenario in machine.ts to the manual case that walks it.
