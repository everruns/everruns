import { isPublicHttpsUrl } from "@/lib/public-origin";

describe("isPublicHttpsUrl", () => {
  it.each([
    "https://example.com/api/v1/e/endpoint/slack/events",
    "https://8.8.8.8/api/v1/e/endpoint/slack/events",
    "https://[2606:4700:4700::1111]/api/v1/e/endpoint/slack/events",
  ])("accepts public HTTPS URLs: %s", (url) => {
    expect(isPublicHttpsUrl(url)).toBe(true);
  });

  it.each([
    "http://example.com/api/v1/e/endpoint/slack/events",
    "https://localhost/api/v1/e/endpoint/slack/events",
    "https://dev.localhost/api/v1/e/endpoint/slack/events",
    "https://0.0.0.0/api/v1/e/endpoint/slack/events",
    "https://127.0.0.2/api/v1/e/endpoint/slack/events",
    "https://10.0.0.1/api/v1/e/endpoint/slack/events",
    "https://100.64.0.1/api/v1/e/endpoint/slack/events",
    "https://172.31.255.255/api/v1/e/endpoint/slack/events",
    "https://192.168.1.1/api/v1/e/endpoint/slack/events",
    "https://169.254.1.1/api/v1/e/endpoint/slack/events",
    "https://198.18.0.1/api/v1/e/endpoint/slack/events",
    "https://203.0.113.1/api/v1/e/endpoint/slack/events",
    "https://224.0.0.1/api/v1/e/endpoint/slack/events",
    "https://[::]/api/v1/e/endpoint/slack/events",
    "https://[::1]/api/v1/e/endpoint/slack/events",
    "https://[fc00::1]/api/v1/e/endpoint/slack/events",
    "https://[fd12:3456:789a::1]/api/v1/e/endpoint/slack/events",
    "https://[fe80::1]/api/v1/e/endpoint/slack/events",
    "https://[fec0::1]/api/v1/e/endpoint/slack/events",
    "https://[ff02::1]/api/v1/e/endpoint/slack/events",
    "https://[2001:db8::1]/api/v1/e/endpoint/slack/events",
    "https://[::ffff:127.0.0.2]/api/v1/e/endpoint/slack/events",
  ])("rejects non-public origins: %s", (url) => {
    expect(isPublicHttpsUrl(url)).toBe(false);
  });
});
