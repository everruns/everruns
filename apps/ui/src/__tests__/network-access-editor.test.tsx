import { render, screen, fireEvent } from "@testing-library/react";
import {
  NetworkAccessEditor,
  buildNetworkAccess,
  findInvalidPatterns,
  normalizeNetworkAccess,
  parseNetworkAccessPatterns,
} from "@/components/network-access-editor";

describe("parseNetworkAccessPatterns", () => {
  it("splits lines, trims, and drops empties", () => {
    expect(parseNetworkAccessPatterns("a.com\n  *.b.com  \n\n\nc.com\n")).toEqual([
      "a.com",
      "*.b.com",
      "c.com",
    ]);
  });

  it("returns empty list for blank input", () => {
    expect(parseNetworkAccessPatterns("")).toEqual([]);
    expect(parseNetworkAccessPatterns("  \n \n")).toEqual([]);
  });
});

describe("buildNetworkAccess", () => {
  it("includes only non-empty lists", () => {
    expect(buildNetworkAccess("a.com", "")).toEqual({ allowed: ["a.com"] });
    expect(buildNetworkAccess("", "b.com")).toEqual({ blocked: ["b.com"] });
    expect(buildNetworkAccess("a.com", "b.com")).toEqual({
      allowed: ["a.com"],
      blocked: ["b.com"],
    });
  });

  it("returns an empty object when both lists are empty (clears the layer)", () => {
    expect(buildNetworkAccess("", "")).toEqual({});
  });
});

describe("findInvalidPatterns", () => {
  it("flags patterns with whitespace or non-http schemes", () => {
    expect(findInvalidPatterns(["a b.com", "ftp://x.com", "ok.com"])).toEqual([
      "a b.com",
      "ftp://x.com",
    ]);
  });

  it("accepts domains, wildcards, and http(s) URL prefixes", () => {
    expect(
      findInvalidPatterns(["example.com", "*.example.com", "https://example.com/api/"]),
    ).toEqual([]);
  });
});

describe("normalizeNetworkAccess", () => {
  it("treats null, {}, and empty arrays as equivalent", () => {
    expect(normalizeNetworkAccess(null)).toEqual(normalizeNetworkAccess({}));
    expect(normalizeNetworkAccess(undefined)).toEqual(
      normalizeNetworkAccess({ allowed: [], blocked: [] }),
    );
  });
});

describe("NetworkAccessEditor", () => {
  it("renders initial patterns one per line", () => {
    render(
      <NetworkAccessEditor
        value={{ allowed: ["api.example.com", "*.github.com"], blocked: ["evil.com"] }}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByLabelText("Allowed hosts")).toHaveValue("api.example.com\n*.github.com");
    expect(screen.getByLabelText("Blocked hosts")).toHaveValue("evil.com");
  });

  it("emits the parsed value on edit", () => {
    const onChange = jest.fn();
    render(<NetworkAccessEditor value={null} onChange={onChange} />);

    fireEvent.change(screen.getByLabelText("Allowed hosts"), {
      target: { value: "api.example.com\n*.github.com" },
    });
    expect(onChange).toHaveBeenLastCalledWith({
      allowed: ["api.example.com", "*.github.com"],
    });

    fireEvent.change(screen.getByLabelText("Blocked hosts"), {
      target: { value: "internal.corp" },
    });
    expect(onChange).toHaveBeenLastCalledWith({
      allowed: ["api.example.com", "*.github.com"],
      blocked: ["internal.corp"],
    });
  });

  it("emits an empty object when all patterns are removed", () => {
    const onChange = jest.fn();
    render(<NetworkAccessEditor value={{ allowed: ["a.com"] }} onChange={onChange} />);

    fireEvent.change(screen.getByLabelText("Allowed hosts"), { target: { value: "" } });
    expect(onChange).toHaveBeenLastCalledWith({});
  });

  it("shows a validation hint for invalid patterns", () => {
    render(<NetworkAccessEditor value={null} onChange={jest.fn()} />);

    fireEvent.change(screen.getByLabelText("Allowed hosts"), {
      target: { value: "bad pattern.com" },
    });
    expect(screen.getByText(/Invalid pattern/)).toBeInTheDocument();
  });

  it("disables both textareas when disabled", () => {
    render(<NetworkAccessEditor value={null} onChange={jest.fn()} disabled />);

    expect(screen.getByLabelText("Allowed hosts")).toBeDisabled();
    expect(screen.getByLabelText("Blocked hosts")).toBeDisabled();
  });
});
