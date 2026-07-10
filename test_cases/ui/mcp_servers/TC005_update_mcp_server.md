# TC005: Update MCP Server

## Description

Verify that an existing MCP server's name, description, and URL can be edited from the MCP Servers list.

## Preconditions

- API server is running
- An active MCP server exists with name "test-mcp-server"

## Test Data

| Field       | Original Value                | Updated Value                  |
|-------------|-------------------------------|--------------------------------|
| Name        | test-mcp-server              | updated-mcp-server             |
| Description | Original description          | Updated description            |
| URL         | https://old.mcp.com/v1/mcp   | https://new.mcp.com/v1/mcp    |

## Steps

1. Navigate to Building blocks > MCP Servers
2. Find the row for the server "test-mcp-server"
3. Click the "Edit" button in that row
4. In the "Edit MCP Server" dialog, update Name to: `updated-mcp-server`
5. Update Description to: `Updated description`
6. Update URL to: `https://new.mcp.com/v1/mcp`
7. Click the "Save" button

## Expected Result

- The dialog closes and the MCP server is updated successfully
- The list row shows the updated name, and the updated description/URL appear in the row subtitle
- `updated_at` timestamp is updated
- Submitting an invalid URL keeps the dialog open and shows "URL must be a valid absolute URL"
- A backend failure (e.g. a duplicate name) keeps the dialog open and surfaces the error message inline

## Notes

- Authentication mode is managed separately: use the "Set Key" action to update an API key. The Edit dialog does not change auth mode.
</content>
