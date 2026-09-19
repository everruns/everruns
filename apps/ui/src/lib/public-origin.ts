function parseIpv4(hostname: string): number[] | null {
  const parts = hostname.split(".");
  if (parts.length !== 4) return null;

  const octets = parts.map((part) => (/^\d+$/.test(part) ? Number(part) : Number.NaN));
  return octets.every((octet) => Number.isInteger(octet) && octet >= 0 && octet <= 255)
    ? octets
    : null;
}

function isPublicIpv4(octets: number[]): boolean {
  const [a, b, c] = octets;
  if (a === 0 || a === 10 || a === 127 || a >= 224) return false;
  if (a === 100 && b >= 64 && b <= 127) return false;
  if (a === 169 && b === 254) return false;
  if (a === 172 && b >= 16 && b <= 31) return false;
  if (a === 192 && b === 0 && (c === 0 || c === 2)) return false;
  if (a === 192 && b === 88 && c === 99) return false;
  if (a === 192 && b === 168) return false;
  if (a === 198 && (b === 18 || b === 19)) return false;
  if (a === 198 && b === 51 && c === 100) return false;
  if (a === 203 && b === 0 && c === 113) return false;
  return true;
}

function parseIpv6(hostname: string): number[] | null {
  const unwrapped =
    hostname.startsWith("[") && hostname.endsWith("]") ? hostname.slice(1, -1) : hostname;
  const halves = unwrapped.split("::");
  if (halves.length > 2) return null;

  const left = halves[0] ? halves[0].split(":") : [];
  const right = halves[1] ? halves[1].split(":") : [];
  const hasCompression = halves.length === 2;
  const missing = 8 - left.length - right.length;
  if ((!hasCompression && missing !== 0) || (hasCompression && missing < 1)) return null;

  const parts = hasCompression ? [...left, ...Array(missing).fill("0"), ...right] : left;
  if (parts.length !== 8 || parts.some((part) => !/^[0-9a-f]{1,4}$/i.test(part))) {
    return null;
  }

  return parts.map((part) => Number.parseInt(part, 16));
}

function isPublicIpv6(hextets: number[]): boolean {
  const isIpv4Mapped = hextets.slice(0, 5).every((hextet) => hextet === 0) && hextets[5] === 0xffff;
  if (isIpv4Mapped) {
    const ipv4 = (hextets[6] << 16) | hextets[7];
    return isPublicIpv4([
      (ipv4 >>> 24) & 0xff,
      (ipv4 >>> 16) & 0xff,
      (ipv4 >>> 8) & 0xff,
      ipv4 & 0xff,
    ]);
  }
  if ((hextets[0] & 0xe000) !== 0x2000) return false;
  if (hextets[0] === 0x2001 && hextets[1] === 0x0db8) return false;
  if (hextets[0] === 0x2001 && hextets[1] === 0x0002 && hextets[2] === 0) return false;
  if (
    hextets[0] === 0x2001 &&
    ((hextets[1] & 0xfff0) === 0x0010 || (hextets[1] & 0xfff0) === 0x0020)
  ) {
    return false;
  }
  return true;
}

export function isPublicHttpsUrl(value: string | null): boolean {
  if (!value) return false;

  try {
    const url = new URL(value);
    const hostname = url.hostname.toLowerCase();
    if (url.protocol !== "https:" || hostname === "localhost" || hostname.endsWith(".localhost")) {
      return false;
    }

    const ipv4 = parseIpv4(hostname);
    if (ipv4) return isPublicIpv4(ipv4);

    const ipv6 = parseIpv6(hostname);
    if (ipv6 !== null) return isPublicIpv6(ipv6);

    return true;
  } catch {
    return false;
  }
}
