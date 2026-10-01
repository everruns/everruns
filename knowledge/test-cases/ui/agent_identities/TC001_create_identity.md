---
type: Test Case
title: "TC001: Create Virtual User"
description: "Verify that a new service virtual user can be created with locale and timezone defaults."
tags:
  - everruns
  - test-case
  - ui
  - virtual-users
---
# TC001: Create Virtual User

## Description

Verify that a new service virtual user can be created with locale and timezone defaults.

## Preconditions

- UI running (dev or full mode)
- User is authenticated

## Test Data

| Field | Value |
|-------|-------|
| Display Name | Test Identity |
| Locale | en-US |
| Timezone | America/New_York |

## Steps

1. Navigate to `/virtual-users/new`
2. Fill in display name, locale, and timezone
3. Submit the form

## Expected Result

- Identity is created successfully
- Identity appears in the identities list with correct locale and timezone defaults
