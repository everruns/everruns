// Drives pi-poppy (github.com/richardanaya/pi-poppy), an independent Poppy
// personal agent, against an Everruns Poppy channel. Run by the domain test
// `pi_poppy_interop_passes` through `scripts/poppy-interop.sh`.
//
// Design Decisions:
// - Every protocol step is pi-poppy's own code (`PersonalAgent`), so its
//   parsers and checks judge our wire format. This file only supplies what a
//   company website and a network would: an HTTPS front on a company domain
//   whose `/.well-known/poppy.json` redirects to the channel, and name
//   resolution for that domain.
// - pi-poppy publishes its client metadata on loopback, which our server
//   refuses to fetch (spec guides: private addresses only when a deployment
//   allows them). The test reads the documents from the `IDENTITY` line and
//   serves them to the server's document cache instead; the server still
//   checks them as it would after a fetch.
import assert from "node:assert/strict";
import { execFile, execFileSync } from "node:child_process";
import dns from "node:dns";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { request as httpForward } from "node:http";
import { createServer } from "node:https";
import { syncBuiltinESMExports } from "node:module";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";

const piPoppy = required("PI_POPPY_DIR");
const upstream = new URL(required("CHANNEL_SERVER")); // http://127.0.0.1:port
const channelId = required("CHANNEL_ID");
const domain = required("COMPANY_DOMAIN"); // e.g. poppy.localhost

// The company domain resolves to loopback, as `*.localhost` does in browsers.
// pi-poppy imports `dns/promises` by name, so the builtin's ESM exports are
// synced after patching, before pi-poppy is imported.
const toLoopback = (host: string) => (host === domain ? "127.0.0.1" : host);
const lookup = dns.lookup;
(dns as { lookup: unknown }).lookup = (host: string, ...rest: unknown[]) =>
	(lookup as (...args: unknown[]) => unknown)(toLoopback(host), ...rest);
const lookupPromise = dns.promises.lookup;
(dns.promises as { lookup: unknown }).lookup = (host: string, ...rest: unknown[]) =>
	(lookupPromise as (...args: unknown[]) => unknown)(toLoopback(host), ...rest);
syncBuiltinESMExports();

const { PersonalAgent } = await import(`${piPoppy}/src/agent.ts`);
const { IdentityServer } = await import(`${piPoppy}/src/identity.ts`);
const { generateEs256 } = await import(`${piPoppy}/src/jwt.ts`);
const { MemoryStore } = await import(`${piPoppy}/src/store.ts`);
const { createTestMaterial } = await import(`${piPoppy}/src/tls.ts`);
const { httpRequest } = await import(`${piPoppy}/src/net.ts`);

const company = companyCertificate(domain);
const front = createServer({ cert: company.cert, key: company.key }, (req, res) => {
	const path = req.url ?? "/";
	if (req.method === "GET" && path === "/.well-known/poppy.json") {
		// What the company's own website does (spec 3).
		res.writeHead(302, {
			location: `https://${req.headers.host}/api/v1/channels/${channelId}/poppy/poppy.json?domain=${domain}`,
		});
		res.end();
		return;
	}
	const forward = httpForward(
		{
			host: upstream.hostname,
			port: upstream.port,
			method: req.method,
			path,
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
const companyUrl = `https://${domain}:${(front.address() as AddressInfo).port}`;

const tls = createTestMaterial();
const keys = generateEs256();
const identity = new IdentityServer({
	cert: tls.certPem,
	key: tls.keyPem,
	clientName: "Pi Poppy",
	signingPublicJwk: keys.publicJwk,
	kid: "agent-1",
});
await identity.start();
const fetchJson = async (url: string) =>
	JSON.parse((await httpRequest(url, { ca: tls.caPem, allowLoopback: true })).body.toString("utf8"));
const metadata = await fetchJson(identity.clientId);
const jwks = await fetchJson(metadata.jwks_uri);
console.log(`IDENTITY ${JSON.stringify({ client_id: identity.clientId, metadata, jwks_uri: metadata.jwks_uri, jwks })}`);
await new Promise<void>((resolve) => {
	const lines = createInterface({ input: process.stdin });
	lines.on("line", (line) => {
		if (line.trim() === "go") {
			lines.close();
			resolve();
		}
	});
});

const agent = new PersonalAgent({
	clientId: identity.clientId,
	clientName: "Pi Poppy",
	redirectUris: [identity.redirectUri],
	signingPrivateJwk: keys.privateJwk,
	signingKid: "agent-1",
	store: new MemoryStore(),
	ca: company.ca,
	allowLoopback: true,
});

let failed = false;
async function step(name: string, run: () => Promise<void>) {
	try {
		await run();
		console.log(`PASS ${name}`);
	} catch (error) {
		failed = true;
		console.log(`FAIL ${name}: ${error instanceof Error ? (error.stack ?? error.message) : String(error)}`);
	}
}

let found: any;
let live: any;
await step("discover follows the company redirect and checks the issuer", async () => {
	found = await agent.discover(companyUrl);
	assert.equal(found.document.organization.domain, domain);
});
await step("signed-out Session starts", async () => {
	live = await agent.startSession(found);
	assert.ok(live.accessToken);
});
await step("Session renews", async () => {
	live = await agent.renew(live);
});
let cursor: string | undefined;
await step("first message starts a conversation and the company agent replies", async () => {
	const sent = await agent.sendMessage(live, { id: "msg_interop_1", sender: "agent", text: "Is my order late?" }, { wait: 10 });
	assert.equal(sent.responder, "agent");
	const texts = (sent.events?.events ?? []).map((event: any) => event.message?.text ?? event.type);
	assert.ok(texts.some((text: string) => /Reply/.test(String(text))), `no company reply in ${JSON.stringify(sent.events)}`);
	cursor = sent.events?.cursor;
});
await step("a retried message is accepted once", async () => {
	await agent.sendMessage(live, { id: "msg_interop_1", sender: "agent", text: "Is my order late?" }, { wait: 0 });
});
await step("a human message and reading events from the cursor", async () => {
	await agent.sendMessage(live, { id: "msg_interop_2", sender: "human", text: "Thanks!" }, { wait: 0 });
	const events = await agent.readEvents(live, { wait: 10, cursor });
	assert.ok(Array.isArray(events.events));
});
await step("handoff", async () => {
	const handed = await agent.handoff(live);
	assert.ok(handed.status);
});
await step("close", async () => {
	const closed = await agent.closeConversation(live);
	assert.equal(closed.status, "closed");
});
await step("revoke ends the Session", async () => {
	await agent.revoke(live);
});

const papPython = process.env.PAP_PYTHON;
if (papPython) {
	await step("Datalayer pap company verify accepts discovery", async () => {
		const caFile = join(mkdtempSync(join(tmpdir(), "poppy-ca-")), "ca.pem");
		writeFileSync(caFile, company.ca);
		const script = new URL("./pap_verify.py", import.meta.url).pathname;
		const output = await new Promise<string>((resolve, reject) => {
			execFile(
				papPython,
				[script],
				{ env: { ...process.env, FRONT_PORT: String((front.address() as AddressInfo).port), COMPANY_CA_FILE: caFile } },
				(error, stdout, stderr) => (error ? reject(new Error(`${stdout}${stderr}`)) : resolve(stdout)),
			);
		});
		assert.match(output, /^PASS/m, output);
	});
}

await identity.close();
front.close();
process.exit(failed ? 1 : 0);

function required(name: string): string {
	const value = process.env[name];
	if (!value) throw new Error(`${name} is required`);
	return value;
}

/// A throwaway CA and a certificate for `host`, like pi-poppy's own test material.
function companyCertificate(host: string): { cert: string; key: string; ca: string } {
	const dir = mkdtempSync(join(tmpdir(), "poppy-company-"));
	const file = (name: string) => join(dir, name);
	const run = (...args: string[]) => execFileSync("openssl", args, { stdio: "ignore" });
	run("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", file("ca.key"), "-out", file("ca.pem"), "-days", "1", "-subj", "/CN=poppy interop CA", "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign");
	run("req", "-newkey", "rsa:2048", "-nodes", "-keyout", file("key.pem"), "-out", file("csr.pem"), "-subj", `/CN=${host}`);
	writeFileSync(file("ext"), `subjectAltName=DNS:${host}\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n`);
	run("x509", "-req", "-in", file("csr.pem"), "-CA", file("ca.pem"), "-CAkey", file("ca.key"), "-CAcreateserial", "-out", file("cert.pem"), "-days", "1", "-extfile", file("ext"));
	return { cert: readFileSync(file("cert.pem"), "utf8"), key: readFileSync(file("key.pem"), "utf8"), ca: readFileSync(file("ca.pem"), "utf8") };
}
