// Fixture-only DNS: preserve the issuer hostname and TLS verification while
// keeping every IdP request on loopback. Never loaded by production startup.
const dns = require("node:dns");
const lookup = dns.lookup;
dns.lookup = function (hostname, options, callback) {
  if (hostname !== "capacity-bench.invalid")
    return lookup.apply(this, arguments);
  if (typeof options === "function") {
    callback = options;
    options = {};
  }
  process.nextTick(() => {
    if (options?.all) callback(null, [{ address: "127.0.0.1", family: 4 }]);
    else callback(null, "127.0.0.1", 4);
  });
};
