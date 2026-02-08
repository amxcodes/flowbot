---
name: notion
description: "Manage Notion pages and databases via API"
category: productivity
status: active
---

# Notion Skill

Interact with Notion workspace - create pages, query databases, update content.

## Prerequisites

1. Create a Notion integration at [Notion Integrations](https://www.notion.so/my-integrations)
2. Share your pages/databases with the integration
3. Set API key:
   ```bash
   export NOTION_API_KEY="secret_xxx"
   ```

## Tools Provided

### `notion_page_create`
Create a new page.
- **Args**: `parent_id` (page/database ID), `title`, `content` (markdown)
- **Example**: `{"tool": "notion_page_create", "parent_id": "abc123", "title": "Meeting Notes", "content": "# Agenda\n- Topic 1"}`

### `notion_page_read`
Read page content.
- **Args**: `page_id`
- **Example**: `{"tool": "notion_page_read", "page_id": "xyz789"}`
- **Returns**: Page title and content as markdown

### `notion_database_query`
Query a database.
- **Args**: `database_id`, `filter` (optional JSON), `limit` (default: 10)
- **Example**: `{"tool": "notion_database_query", "database_id": "db123", "limit": 5}`

### `notion_page_update`
Update page content.
- **Args**: `page_id`, `content` (markdown to append/replace)
- **Example**: `{"tool": "notion_page_update", "page_id": "abc", "content": "Updated text"}`

## Configuration

```toml
[skills.notion]
enabled = true
api_key = "${NOTION_API_KEY}"
version = "2022-06-28"  # Notion API version
```

## Usage Examples

**Create a page:**
```
> Create a Notion page titled "Project Ideas" in my workspace
✓ Created page: Project Ideas
  URL: https://notion.so/Project-Ideas-abc123
```

**Query database:**
```
> Show me the latest 5 entries from my Tasks database (id: db_abc)
✓ Found 5 tasks:
  1. [Todo] Fix homepage bug
  2. [In Progress] Write documentation
  3. [Done] Deploy backend
```

**Read a page:**
```
> Read the content of Notion page xyz789
✓ Page: Weekly Report
  Content: [markdown rendered]
```

## Implementation Notes

- Uses Notion API v2022-06-28
- Requires integration to be shared with pages/databases
- Supports rich text, markdown conversion
- Rate limit: 3 requests/second (cloud API)
