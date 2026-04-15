export const authQueryKeys = {
  all: ["auth"] as const,
  config: () => [...authQueryKeys.all, "config"] as const,
  user: () => [...authQueryKeys.all, "user"] as const,
  apiKeys: (org?: string) =>
    org
      ? ([...authQueryKeys.all, "api-keys", org] as const)
      : ([...authQueryKeys.all, "api-keys"] as const),
};
