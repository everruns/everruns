export const authQueryKeys = {
  all: ["auth"] as const,
  config: () => [...authQueryKeys.all, "config"] as const,
  user: () => [...authQueryKeys.all, "user"] as const,
  personalAccessTokens: (org?: string) =>
    org
      ? ([...authQueryKeys.all, "personal-access-tokens", org] as const)
      : ([...authQueryKeys.all, "personal-access-tokens"] as const),
};
