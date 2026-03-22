# TC005: Invalid Image Type Rejected

## Description

Verify that non-image files and unsupported image formats are rejected with clear error messages.

## Preconditions

- Server running (`just start-all`)
- User logged in
- An active session open in the chat view

## Test Data

| Field | Value |
|-------|-------|
| Invalid file | A .txt, .pdf, or .svg file |

## Steps

1. Navigate to a session chat view
2. Click the image attach button
3. Attempt to select a non-image file (change file picker filter to "All Files" if needed)
4. Observe behavior

## Expected Result

| Check | Expected |
|-------|----------|
| File picker filter | Only shows PNG, JPEG, GIF, WebP by default |
| Invalid file | Rejected with validation error in console |
| No upload | No upload request sent for invalid files |
