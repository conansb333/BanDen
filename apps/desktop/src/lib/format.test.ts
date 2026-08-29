import { describe, expect, it } from "vitest";
import { formatBps, formatBytes, formatDuration, relativeTime } from "./format";

describe("formatBytes", () => {
  it("formats SI-friendly byte counts", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatBytes(3.5 * 1024 ** 3)).toBe("3.5 GB");
  });
});

describe("formatBps", () => {
  it("formats bit rates with decimal units", () => {
    expect(formatBps(0)).toBe("0 bps");
    expect(formatBps(999)).toBe("999 bps");
    expect(formatBps(82_000_000)).toBe("82.0 Mbps");
    expect(formatBps(2_500_000)).toBe("2.5 Mbps");
  });
});

describe("formatDuration", () => {
  it("formats elapsed seconds compactly", () => {
    expect(formatDuration(42)).toBe("42s");
    expect(formatDuration(125)).toBe("2m 5s");
    expect(formatDuration(7265)).toBe("2h 1m");
  });
});

describe("relativeTime", () => {
  it("describes recency", () => {
    const now = Date.now();
    expect(relativeTime(new Date(now - 1000).toISOString(), now)).toBe("just now");
    expect(relativeTime(new Date(now - 30_000).toISOString(), now)).toBe("30s ago");
    expect(relativeTime(new Date(now - 120_000).toISOString(), now)).toBe("2m ago");
  });
});
