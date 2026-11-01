// What a host calls the charges it reports for usage it billed itself.
//
// Decision: OSS has no credits. A host (the hosted SaaS) overrides this file
// through its path alias to say "Billed in Everruns credits"; self-hosted
// Everruns says the neutral "Billed by host", which only appears when a host
// integration reports charges at all.
export const HOST_BILLED_LABEL = "Billed by host";
export const HOST_BILLED_DESCRIPTION =
  "Your host reported what it charged for this usage. It replaces any estimate.";
