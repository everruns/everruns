"use client";

import { useEffect, useState } from "react";
import type { Viewer } from "../profile";

function initials(name: string) {
  const parts = name.split(/\s+/).filter(Boolean);
  const words = parts.length > 1 ? parts.slice(0, 2) : parts.slice(0, 1);
  return words
    .map((part) => [...part][0] ?? "")
    .join("")
    .toUpperCase();
}
export function Account({ viewer }: { viewer: Viewer }) {
  const [picture, setPicture] = useState(viewer.picture);
  useEffect(() => setPicture(viewer.picture), [viewer.picture]);
  return (
    <div className="account">
      {picture ? (
        <img
          src={picture}
          alt=""
          referrerPolicy="no-referrer"
          onError={() => setPicture(undefined)}
        />
      ) : (
        <span className="avatar" aria-hidden="true">
          {initials(viewer.name)}
        </span>
      )}
      <span
        className="account-name"
        title={viewer.email ? `${viewer.name} ${viewer.email}` : viewer.name}
      >
        <strong>{viewer.name}</strong>
        {viewer.email ? <small>{viewer.email}</small> : null}
      </span>
    </div>
  );
}
