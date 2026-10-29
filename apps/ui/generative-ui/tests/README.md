`standard-samples.json` contains the 39 catalog samples covering all 69 standard
components from CopilotKit's `showcase/shared/generative-ui-catalog/samples.ts`
at revision `ea0aa986e32cf7aeec46e7004bff8e48a81c6550`.
The upstream MIT notice is retained in `../COPILOTKIT-LICENSE`.

The same fixtures exercise the production renderer through both chat hosts.
`protocol.test.ts` runs with Node's TypeScript stripping, avoiding tsx's
resolution of an unpublished TypeScript entry in fast-json-patch.
