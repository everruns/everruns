/**
 * Copy for providers the deployment owns (`provider.managed`).
 *
 * Decision: keep these strings in one tiny module so a distribution can
 * override the file wholesale via the `@/*` path alias and describe the
 * provider in its own words. A self-hosted install frames a managed provider
 * as "the host configured this"; a hosted product frames the same provider as
 * a service it supplies. The underlying read-only behaviour is identical, so
 * only the wording varies.
 */

export const managedProviderCopy = {
  /** Badge label on the provider card and detail header. */
  badge: "Managed",
  /** `title` attribute for that badge. */
  badgeTitle: "Managed by the host",
  /** Notice above the (disabled) settings form on the provider detail page. */
  notice:
    "This provider is managed by the host and is read-only. Its credentials and configuration cannot be changed here.",
};
