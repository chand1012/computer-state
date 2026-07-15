import { describe, expect, it } from "vitest";
import { buildChartData, createTimeBounds, parseStoredCharts } from "./MetricsPage";

describe("buildChartData", () => {
  it("groups labeled series at each timestamp", () => {
    const result = buildChartData([
      { metric: "cpu.core.usage", timestamp: 2000, value: 40, labels: { core: "0" } },
      { metric: "cpu.core.usage", timestamp: 1000, value: 30, labels: { core: "1" } },
      { metric: "cpu.core.usage", timestamp: 1000, value: 20, labels: { core: "0" } },
    ]);
    expect(result.series).toEqual(["0", "1"]);
    expect(result.data).toEqual([
      { timestamp: 1000, "0": 20, "1": 30 },
      { timestamp: 2000, "0": 40 },
    ]);
  });

  it("inserts an empty point instead of connecting across a collection gap", () => {
    const result = buildChartData([
      { metric: "cpu.total.usage", timestamp: 0, value: 20, labels: {} },
      { metric: "cpu.total.usage", timestamp: 60_000, value: 25, labels: {} },
      { metric: "cpu.total.usage", timestamp: 3_660_000, value: 10, labels: {} },
    ], 60_000);

    expect(result.data).toEqual([
      { timestamp: 0, value: 20 },
      { timestamp: 60_000, value: 25 },
      { timestamp: 120_000, value: null },
      { timestamp: 3_660_000, value: 10 },
    ]);
  });

  it("does not add gaps between adjacent aggregation buckets", () => {
    const result = buildChartData([
      { metric: "cpu.total.usage", timestamp: 0, value: 20, labels: {} },
      { metric: "cpu.total.usage", timestamp: 60_000, value: 25, labels: {} },
    ], 60_000);

    expect(result.data).toHaveLength(2);
  });

  it("anchors a fifteen-minute window at the current time", () => {
    expect(createTimeBounds(0.25, 1_000_000)).toEqual({ from: 100_000, to: 1_000_000 });
  });

});

describe("parseStoredCharts", () => {
  it("restores valid chart selections", () => {
    const charts = [{ id: "chart-1", metric: "memory.used", hours: 24 }];
    expect(parseStoredCharts(JSON.stringify(charts))).toEqual(charts);
  });

  it("rejects malformed or unsupported chart selections", () => {
    expect(parseStoredCharts("not json")).toBeUndefined();
    expect(parseStoredCharts(JSON.stringify([{ id: "chart-1", metric: "memory.used", hours: 2 }]))).toBeUndefined();
  });
});
