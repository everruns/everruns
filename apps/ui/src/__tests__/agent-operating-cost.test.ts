import { projectOperatingCosts } from "@/lib/agent-operating-cost";

it("separates prepaid usage from provider bills and charges monthly fees once", () => {
  const result = projectOperatingCosts(
    0.02,
    "credits",
    [
      {
        id: "sandbox",
        label: "Sandbox",
        unit: "minute",
        quantity: 5,
        usdPerUnit: 0.01,
        billing: "external",
        period: "task",
      },
      {
        id: "search",
        label: "Search",
        unit: "search",
        quantity: 2,
        usdPerUnit: 0.006,
        billing: "credits",
        period: "task",
      },
      {
        id: "hosting",
        label: "Hosting",
        unit: "month",
        quantity: 1,
        usdPerUnit: 10,
        billing: "external",
        period: "month",
      },
    ],
    100,
  )!;
  expect(result.credits.monthly).toBeCloseTo(96);
  expect(result.external.monthly).toBeCloseTo(160);
  expect(result.total.monthly).toBeCloseTo(256);
  expect(result.total.fixedMonthly).toBe(10);
  expect(result.complete).toBe(true);
});

it("retains unknown charges as incomplete instead of presenting them as free", () => {
  const lines = [
    {
      id: "sandbox",
      label: "Sandbox",
      unit: "minute",
      quantity: 5,
      billing: "external" as const,
      period: "task" as const,
    },
  ];
  const result = projectOperatingCosts(0.02, "credits", lines, 100)!;
  expect(result.complete).toBe(false);
  expect(result.unknown).toEqual(["Sandbox"]);
  expect(result.external.monthly).toBeNull();
  expect(result.credits.monthly).toBeCloseTo(60);
  expect(result.total.monthly).toBeNull();
  expect(projectOperatingCosts(0.02, "credits", lines, 0)!.total.monthly).toBe(0);
});

it("keeps fixed monthly costs at zero activity and rejects invalid inputs", () => {
  const line = {
    id: "fee",
    label: "Fee",
    unit: "month",
    quantity: 1,
    usdPerUnit: 10,
    billing: "external" as const,
    period: "month" as const,
  };
  expect(projectOperatingCosts(null, "external", [line], 0)!.total.monthly).toBe(10);
  expect(
    projectOperatingCosts(0, "external", [{ ...line, usdPerUnit: 0 }], 10)!.total.monthly,
  ).toBe(0);
  expect(projectOperatingCosts(1, "external", [{ ...line, quantity: NaN }], 10)).toBeNull();
  expect(projectOperatingCosts(1, "external", [{ ...line, usdPerUnit: -1 }], 10)).toBeNull();
  expect(projectOperatingCosts(Number.MAX_VALUE, "external", [], 100)).toBeNull();
});
