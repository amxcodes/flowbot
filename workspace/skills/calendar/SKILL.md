---
name: calendar
description: "Manage Google Calendar events via API"
category: productivity
status: active
---

# Calendar Skill

Manage Google Calendar events - create, view, update, and delete events.

## Prerequisites

1. Enable Google Calendar API in [Google Cloud Console](https://console.cloud.google.com)
2. Create OAuth 2.0 credentials
3. Use Nanobot's OAuth flow:
   ```bash
   nanobot login google-calendar
   ```

## Tools Provided

### `calendar_event_create`
Create a new calendar event.
- **Args**: `summary` (title), `start`, `end`, `description` (optional), `location` (optional)
- **Example**: `{"tool": "calendar_event_create", "summary": "Team Meeting", "start": "2024-03-15T10:00:00", "end": "2024-03-15T11:00:00", "description": "Q1 review"}`

### `calendar_events_list`
List upcoming events.
- **Args**: `days_ahead` (default: 7), `limit` (default: 10)
- **Example**: `{"tool": "calendar_events_list", "days_ahead": 3, "limit": 5}`

### `calendar_event_update`
Update an existing event.
- **Args**: `event_id`, `summary` (optional), `start` (optional), `end` (optional)
- **Example**: `{"tool": "calendar_event_update", "event_id": "abc123", "summary": "Updated Meeting Title"}`

### `calendar_event_delete`
Delete an event.
- **Args**: `event_id`
- **Example**: `{"tool": "calendar_event_delete", "event_id": "xyz789"}`

### `calendar_search`
Search events by keyword.
- **Args**: `query`, `days_ahead` (default: 30)
- **Example**: `{"tool": "calendar_search", "query": "dentist"}`

## Configuration

```toml
[skills.calendar]
enabled = true
calendar_id = "primary"  # or specific calendar ID
timezone = "America/New_York"  # Your timezone
```

## Usage Examples

**List upcoming events:**
```
> What's on my calendar for the next 3 days?
✓ Upcoming events (next 3 days):
  - Today 2:00 PM: Team Standup
  - Tomorrow 10:00 AM: Client Call
  - Mar 16 9:00 AM: Dentist Appointment
```

**Create event:**
```
> Create a calendar event "Lunch with Sarah" tomorrow at 12:30pm for 1 hour
✓ Created event: Lunch with Sarah
  Time: Mar 15, 2024 12:30 PM - 1:30 PM
  Event ID: abc123def456
```

**Search:**
```
> Find all dentist appointments in my calendar
✓ Found 2 events:
  1. Dentist Checkup - Mar 16, 9:00 AM
  2. Dentist Follow-up - Apr 20, 2:00 PM
```

**Update event:**
```
> Move my 2pm meeting today to 3pm
✓ Updated: Team Standup → 3:00 PM - 3:30 PM
```

## Implementation Notes

- Uses Google Calendar API v3 (cloud-based)
- OAuth handled by Nanobot's token manager
- Supports multiple calendars
- Time parsing with natural language (via chrono parser)
- Rate limit: 500 queries/100 seconds per user
