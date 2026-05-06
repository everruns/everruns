# TC003: Edit Volume Name and Description

## Description

Verify that an existing active volume's name and description can be edited from the volume card and that changes are reflected in both the list and detail views.

## Preconditions

- Server running (`just start-all`)
- User logged in
- At least one active volume exists (create one via TC001 if needed)

## Test Data

| Field | Value |
|-------|-------|
| Original name | `tc003-source` |
| New name | `tc003-renamed` |
| New description | `Renamed in TC003` |

## Steps

1. Create a volume named `tc003-source` (via the New Volume dialog) if one is not already present
2. Navigate to `/volumes`
3. Locate the `tc003-source` card and click its **Edit** button
4. In the dialog, change the **Name** to `tc003-renamed`
5. Set the **Description** to `Renamed in TC003`
6. Click **Save Changes**
7. Observe the card in the list
8. Click **Open** on the renamed card to navigate to the detail page

## Expected Result

| Check | Expected |
|-------|----------|
| Dialog closes | Dialog closes after save with no error |
| Card name updates | Card title now reads `tc003-renamed` |
| Card description updates | Card body now reads `Renamed in TC003` |
| Updated timestamp | Card "Updated" relative time refreshes to a recent value |
| Detail page header | `/volumes/<id>` page heading reads `tc003-renamed` |
| Detail description | Overview section shows `Renamed in TC003` |
| Page title | Browser tab title contains `tc003-renamed` |

## Cleanup

- Archive `tc003-renamed` from the detail page or list.
