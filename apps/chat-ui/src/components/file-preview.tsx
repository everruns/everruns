"use client";
import { useState } from "react";
import { Streamdown } from "streamdown";
export type FileContent = {
  name: string;
  path: string;
  is_readonly?: boolean;
  content?: string;
  encoding?: string;
  revision?: string;
};
function bytes(file: FileContent) {
  return file.encoding === "base64"
    ? Uint8Array.from(atob(file.content ?? ""), (char) => char.charCodeAt(0))
    : new TextEncoder().encode(file.content ?? "");
}
export function downloadFile(file: FileContent) {
  const url = URL.createObjectURL(new Blob([bytes(file)]));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = file.name;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
function CSV({ content }: { content: string }) {
  const [search, setSearch] = useState("");
  // Quoted commas, newlines and escaped quotes stay within their cells.
  const rows: string[][] = [];
  let row: string[] = [],
    cell = "",
    quoted = false;
  for (let i = 0; i < content.length && rows.length < 1000; i++) {
    const c = content[i];
    if (c === '"') {
      if (quoted && content[i + 1] === '"') {
        cell += '"';
        i++;
      } else quoted = !quoted;
    } else if (!quoted && (c === "," || c === "\n")) {
      row.push(cell);
      cell = "";
      if (c === "\n") {
        rows.push(row);
        row = [];
      }
    } else if (c !== "\r" || quoted) cell += c;
  }
  if (cell || row.length) {
    row.push(cell);
    rows.push(row);
  }
  return (
    <>
      <input
        aria-label="Filter table"
        placeholder="Filter rows…"
        value={search}
        onChange={(event) => setSearch(event.target.value)}
      />
      <div className="widget-table">
        <table>
          <thead>
            <tr>
              {rows[0]?.map((cell, i) => (
                <th key={i}>{cell}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows
              .slice(1)
              .filter((row) => row.join(" ").toLowerCase().includes(search.toLowerCase()))
              .map((row, i) => (
                <tr key={i}>
                  {row.map((cell, j) => (
                    <td key={j}>{cell}</td>
                  ))}
                </tr>
              ))}
          </tbody>
        </table>
      </div>
      <small>Preview shows up to 1,000 rows. Download the complete file.</small>
    </>
  );
}
export function FilePreview({ file }: { file: FileContent }) {
  const extension = file.name.split(".").pop()?.toLowerCase();
  const image = {
    png: "image/png",
    jpg: "image/jpeg",
    jpeg: "image/jpeg",
    gif: "image/gif",
    webp: "image/webp",
  }[extension ?? ""];
  if (file.encoding === "base64")
    return image ? (
      <img className="file-image" src={`data:${image};base64,${file.content}`} alt={file.name} />
    ) : (
      <p>Download this binary file to view it.</p>
    );
  if (extension === "md") return <Streamdown>{file.content ?? ""}</Streamdown>;
  if (extension === "csv") return <CSV content={file.content ?? ""} />;
  // THREAT[TM-WEB-024]: opaque-origin artifacts have no scripts, network or form actions.
  if (extension === "html" || extension === "htm" || extension === "svg")
    return (
      <iframe
        title={`Preview ${file.name}`}
        sandbox=""
        srcDoc={`<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'">${file.content ?? ""}`}
      />
    );
  return (
    <pre>
      <code>{file.content}</code>
    </pre>
  );
}
