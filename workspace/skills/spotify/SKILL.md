---
name: spotify
description: "Control Spotify playback and search music via Spotify API"
category: productivity
status: active
---

# Spotify Skill

Control Spotify playback, search tracks, and manage playlists via Spotify Web API.

## Prerequisites

1. Create a Spotify app at [Spotify Developer Dashboard](https://developer.spotify.com/dashboard)
2. Get Client ID and Client Secret
3. Authorize with OAuth (handled by skill)
4. Set credentials:
   ```bash
   export SPOTIFY_CLIENT_ID="your_client_id"
   export SPOTIFY_CLIENT_SECRET="your_secret"
   ```

## Tools Provided

### `spotify_play`
Play a track, album, or playlist.
- **Args**: `query` (search term or URI)
- **Example**: `{"tool": "spotify_play", "query": "Bohemian Rhapsody"}`

### `spotify_pause`
Pause playback.
- **Example**: `{"tool": "spotify_pause"}`

### `spotify_next`
Skip to next track.
- **Example**: `{"tool": "spotify_next"}`

### `spotify_previous`
Go to previous track.
- **Example**: `{"tool": "spotify_previous"}`

### `spotify_search`
Search for tracks, albums, or artists.
- **Args**: `query`, `type` (track/album/artist/playlist), `limit` (default: 5)
- **Example**: `{"tool": "spotify_search", "query": "Taylor Swift", "type": "track", "limit": 3}`

### `spotify_current`
Get currently playing track.
- **Example**: `{"tool": "spotify_current"}`
- **Returns**: Track name, artist, album, progress

### `spotify_volume`
Set volume (0-100).
- **Args**: `level` (0-100)
- **Example**: `{"tool": "spotify_volume", "level": 75}`

## Configuration

```toml
[skills.spotify]
enabled = true
client_id = "${SPOTIFY_CLIENT_ID}"
client_secret = "${SPOTIFY_CLIENT_SECRET}"
redirect_uri = "http://localhost:8888/callback"  # For OAuth
```

## Usage Examples

**Play music:**
```
> Play "Hotel California" by Eagles on Spotify
✓ Now playing: Hotel California - Eagles
```

**Search:**
```
> Search Spotify for Beatles songs
✓ Top 5 results:
  1. Come Together - The Beatles
  2. Let It Be - The Beatles
  3. Here Comes The Sun - The Beatles
```

**Control playback:**
```
> Pause Spotify
✓ Playback paused

> What's currently playing on Spotify?
✓ Now playing: Imagine - John Lennon (2:34 / 3:05)
```

## Implementation Notes

- Uses Spotify Web API (cloud-based)
- Requires active Spotify Premium for playback control
- Free tier can search and view info only
- OAuth flow handled automatically
- Rate limit: Standard Spotify API limits
