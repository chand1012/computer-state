import { describe, expect, it } from "vitest";
import { formatMetric } from "./api";

describe("formatMetric", () => {
  it("formats percentages, bytes, durations, and counts", () => {
    expect(formatMetric(31.45, "percent")).toBe("31.4%");
    expect(formatMetric(1_073_741_824, "bytes")).toBe("1.0 GB");
    expect(formatMetric(90_000, "seconds")).toBe("1d 1h");
    expect(formatMetric(1234.4, "processes")).toBe("1,234");
  });
});
