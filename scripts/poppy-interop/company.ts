// Runs pi-poppy's company (github.com/richardanaya/pi-poppy, `CompanyServer`),
// an independent Poppy company, for an Everruns agent acting as a personal
// agent to talk to. Started by the domain test
// `poppy_agent_talks_to_pi_poppy_company` through `scripts/poppy-interop.sh`.
//
// Design Decisions:
// - The company is pi-poppy's own code, so its checks judge our client:
//   client metadata and keys, assertions, DPoP proofs (with nonces required),
//   and the conversation API. This file only supplies the network: names
//   under `poppy.localhost` that resolve to loopback, one test CA, and an
//   HTTPS front for the Everruns server's client metadata document.
// - The `client_id` has no port, as a deployment's would not. pi-poppy fetches
//   it on 443, so requests to the agent host are pointed at the front's port.
// - Prints `COMPANY {json}` once ready (the company's port and the CA file the
//   test trusts), then runs until stdin closes.
import { execFileSync } from "node:child_process";
import dns from "node:dns";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { request as httpForward } from "node:http";
import https from "node:https";
import { syncBuiltinESMExports } from "node:module";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

const piPoppy = required("PI_POPPY_DIR");
const upstream = new URL(required("AGENT_SERVER")); // http://127.0.0.1:port
const companyDomain = required("COMPANY_DOMAIN"); // shop.poppy.localhost
const agentHost = required("AGENT_HOST"); // agent.poppy.localhost

const toLoopback = (host: string) => (host === companyDomain || host === agentHost ? "127.0.0.1" : host);
const lookup = dns.lookup;
(dns as { lookup: unknown }).lookup = (host: string, ...rest: unknown[]) =>
	(lookup as (...args: unknown[]) => unknown)(toLoopback(host), ...rest);
const lookupPromise = dns.promises.lookup;
(dns.promises as { lookup: unknown }).lookup = (host: string, ...rest: unknown[]) =>
	(lookupPromise as (...args: unknown[]) => unknown)(toLoopback(host), ...rest);
let frontPort = 0;
const httpsRequest = https.request;
(https as { request: unknown }).request = (options: Record<string, unknown>, ...rest: unknown[]) =>
	(httpsRequest as (...args: unknown[]) => unknown)(
		options.hostname === agentHost ? { ...options, port: frontPort } : options,
		...rest,
	);
syncBuiltinESMExports();

const { CompanyServer } = await import(`${piPoppy}/src/company.ts`);

const tls = certificates([companyDomain, agentHost]);

// The deployment's public address: its client metadata and JWKS.
const front = https.createServer({ cert: tls.cert, key: tls.key }, (req, res) => {
	const forward = httpForward(
		{
			host: upstream.hostname,
			port: upstream.port,
			method: req.method,
			path: req.url ?? "/",
			headers: { ...req.headers, "x-forwarded-proto": "https" },
		},
		(reply) => {
			res.writeHead(reply.statusCode ?? 502, reply.headers);
			reply.pipe(res);
		},
	);
	forward.on("error", () => {
		res.writeHead(502);
		res.end();
	});
	req.pipe(forward);
});
await new Promise<void>((resolve) => front.listen(0, "127.0.0.1", resolve));
frontPort = (front.address() as AddressInfo).port;

const company = new CompanyServer({
	cert: tls.cert,
	key: tls.key,
	ca: tls.ca,
	allowLoopback: true,
	domain: companyDomain,
	requireNonce: true,
});
await company.start();

console.log(`COMPANY ${JSON.stringify({ port: Number(new URL(company.issuer).port), ca_file: tls.caFile })}`);

process.stdin.resume();
process.stdin.on("end", async () => {
	await company.close();
	front.close();
	process.exit(0);
});

function certificates(hosts: string[]): { cert: string; key: string; ca: string; caFile: string } {
	const dir = mkdtempSync(join(tmpdir(), "poppy-company-"));
	const file = (name: string) => join(dir, name);
	const run = (...args: string[]) => execFileSync("openssl", args, { stdio: "ignore" });
	run("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", file("ca.key"), "-out", file("ca.pem"), "-days", "1", "-subj", "/CN=poppy interop CA", "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign");
	run("req", "-newkey", "rsa:2048", "-nodes", "-keyout", file("key.pem"), "-out", file("csr.pem"), "-subj", `/CN=${hosts[0]}`);
	const names = [...hosts.map((host) => `DNS:${host}`), "IP:127.0.0.1"].join(",");
	writeFileSync(file("ext"), `subjectAltName=${names}\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n`);
	run("x509", "-req", "-in", file("csr.pem"), "-CA", file("ca.pem"), "-CAkey", file("ca.key"), "-CAcreateserial", "-out", file("cert.pem"), "-days", "1", "-extfile", file("ext"));
	return { cert: readFileSync(file("cert.pem"), "utf8"), key: readFileSync(file("key.pem"), "utf8"), ca: readFileSync(file("ca.pem"), "utf8"), caFile: file("ca.pem") };
}

function required(name: string): string {
	const value = process.env[name];
	if (!value) {
		console.error(`${name} is required`);
		process.exit(2);
	}
	return value;
}
