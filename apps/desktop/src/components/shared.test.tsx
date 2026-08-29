import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { DeviceStatusBadge, SessionStateBadge } from "./shared";

describe("SessionStateBadge", () => {
  it("renders active as success", () => {
    render(<SessionStateBadge state="active" />);
    expect(screen.getByText("active")).toBeInTheDocument();
  });

  it("renders recovery_required as destructive", () => {
    render(<SessionStateBadge state="recovery_required" />);
    expect(screen.getByText("recovery required")).toBeInTheDocument();
  });
});

describe("DeviceStatusBadge", () => {
  it("renders online", () => {
    render(<DeviceStatusBadge status="online" />);
    expect(screen.getByText("ONLINE")).toBeInTheDocument();
  });

  it("renders unknown fallback", () => {
    render(<DeviceStatusBadge status={"bogus" as never} />);
    expect(screen.getByText("UNKNOWN")).toBeInTheDocument();
  });
});
