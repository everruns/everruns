# TC003: Upload Image via File Picker

## Description

Verify that clicking the image attach button opens a file picker and uploads selected images.

## Preconditions

- Server running (`just start-all`)
- User logged in
- An active session open in the chat view

## Test Data

| Field | Value |
|-------|-------|
| Image file | Any PNG, JPEG, GIF, or WebP file |

## Steps

1. Navigate to a session chat view
2. Click the image attach button (ImagePlus icon) below the message input
3. Select one or more image files from the file picker dialog
4. Observe the image attachment area

## Expected Result

| Check | Expected |
|-------|----------|
| File picker | Opens with image type filter |
| Image thumbnails | Appear in attachment area |
| Upload completes | All images show preview (no error) |
| Multiple images | Up to 10 images can be attached |
