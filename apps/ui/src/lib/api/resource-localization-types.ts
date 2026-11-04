// Additive display metadata over the legacy API compatibility interfaces.
import type { EntityLocalizations } from "../entity-localization";
declare module "./legacy-api-types" {
  interface Harness {
    localizations?: EntityLocalizations;
  }
  interface CreateHarnessRequest {
    localizations?: EntityLocalizations;
  }
  interface UpdateHarnessRequest {
    localizations?: EntityLocalizations;
  }
  interface HarnessExample {
    localizations?: EntityLocalizations;
  }
}
export {};
