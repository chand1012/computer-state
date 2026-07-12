import { describe, expect, it } from "vitest";
import { buildChartData } from "./MetricsPage";

describe("buildChartData", () => {
  it("groups labeled series at each timestamp", () => {
    const result = buildChartData([
      { metric: "cpu.core.usage", timestamp: 1000, value: 20, labels: { core: "0" } },
      { metric: "cpu.core.usage", timestamp: 1000, value: 30, labels: { core: "1" } },
      { metric: "cpu.core.usage", timestamp: 2000, value: 40, labels: { core: "0" } },
    ]);
    expect(result.series).toEqual(["0", "1"]);
    expect(result.data).toEqual([
      { timestamp: 1000, "0": 20, "1": 30 },
      { timestamp: 2000, "0": 40 },
    ]);
  });
});
