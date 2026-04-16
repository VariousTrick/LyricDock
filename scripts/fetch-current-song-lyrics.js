const fs = require('fs/promises');
const path = require('path');
const { execFileSync } = require('child_process');
const crypto = require('crypto');
const netease = require('NeteaseCloudMusicApi');

function readTrackFromEnv() {
  const raw = process.env.LYRICDOCK_TRACK_JSON;
  if (!raw) return null;

  try {
    const parsed = JSON.parse(raw);
    if (
      parsed &&
      typeof parsed.title === 'string' &&
      Array.isArray(parsed.artists) &&
      parsed.artists.length > 0
    ) {
      return {
        title: parsed.title,
        album: typeof parsed.album === 'string' ? parsed.album : '',
        artists: parsed.artists.map((item) => String(item)).filter(Boolean),
        durationMs: Number(parsed.duration_ms ?? parsed.durationMs ?? 0),
      };
    }
  } catch (_) {
  }

  return null;
}

function readSpotifyMetadata() {
  const output = execFileSync(
    'gdbus',
    [
      'introspect',
      '--session',
      '--dest',
      'org.mpris.MediaPlayer2.spotify',
      '--object-path',
      '/org/mpris/MediaPlayer2',
    ],
    { encoding: 'utf8' }
  );

  const title = output.match(/'xesam:title': <'([^']+)'/u)?.[1];
  const album = output.match(/'xesam:album': <'([^']+)'/u)?.[1];
  const durationUs = Number(
    output.match(/'mpris:length': <uint64 (\d+)>/u)?.[1] || 0
  );
  const artistsBlock = output.match(/'xesam:artist': <\[(.*?)\]>/u)?.[1] || '';
  const artists = Array.from(artistsBlock.matchAll(/'([^']+)'/gu)).map(
    (match) => match[1]
  );

  if (!title || artists.length === 0) {
    throw new Error('Could not read current Spotify metadata from MPRIS.');
  }

  return {
    title,
    album: album || '',
    artists,
    durationMs: Math.round(durationUs / 1000),
  };
}

function normalize(text) {
  return text
    .normalize('NFKC')
    .replace(/\s+/g, ' ')
    .trim();
}

function normalizeForMatch(text) {
  return normalize(text)
    .replace(/\([^)]*\)/g, ' ')
    .replace(/\[[^\]]*\]/g, ' ')
    .replace(/\s*-\s*.*$/u, ' ')
    .replace(/\b(feat|with|prod)\b\.?.*$/iu, ' ')
    .replace(/連/g, '连')
    .replace(/藉/g, '借')
    .replace(/舊/g, '旧')
    .replace(/沒/g, '没')
    .replace(/\s+/g, ' ')
    .trim();
}

function sanitizeFileSegment(text) {
  return normalize(text)
    .replace(/[\\/:*?"<>|]/g, '-')
    .replace(/\s+/g, ' ')
    .replace(/-+/g, '-')
    .trim();
}

async function pickAvailableBaseName(outDir, preferredBaseName, uniqueSuffix) {
  const lrcPath = path.join(outDir, `${preferredBaseName}.lrc`);
  const jsonPath = path.join(outDir, `${preferredBaseName}.json`);
  try {
    await fs.access(lrcPath);
    await fs.access(jsonPath);
    return `${preferredBaseName} (${uniqueSuffix})`;
  } catch {
    return preferredBaseName;
  }
}

function scoreCandidate(track, song) {
  let score = 0;
  const title = normalizeForMatch(track.title);
  const artist = normalizeForMatch(track.artists[0]);
  const album = normalizeForMatch(track.album);
  const songTitle = normalizeForMatch(song.name || '');
  const songArtist = normalizeForMatch(
    (song.artists || []).map((item) => item.name).join(' / ')
  );
  const songAlbum = normalizeForMatch(song.album?.name || '');
  const durationDelta = Math.abs((song.duration || 0) - track.durationMs);

  if (songTitle === title) score += 60;
  else if (songTitle.includes(title) || title.includes(songTitle)) score += 40;

  if (songArtist.includes(artist) || artist.includes(songArtist)) score += 50;
  else score -= 40;

  if (album && songAlbum === album) score += 30;
  else if (album && songAlbum && (songAlbum.includes(album) || album.includes(songAlbum))) {
    score += 10;
  } else if (album && songAlbum) {
    score -= 10;
  }

  if (durationDelta <= 1000) score += 40;
  else if (durationDelta <= 3000) score += 25;
  else if (durationDelta <= 8000) score += 10;

  const name = normalize(song.name || '');
  if (/\b(instrumental|伴奏|纯音乐|钢琴版|吉他版|cover|翻自|翻唱)\b/iu.test(name)) {
    score -= 120;
  }

  return score;
}

async function withRetry(fn, attempts = 3) {
  let lastError;
  for (let index = 0; index < attempts; index += 1) {
    try {
      return await fn();
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 300 * (index + 1)));
    }
  }
  throw lastError;
}

async function main() {
  const track = readTrackFromEnv() || readSpotifyMetadata();
  const query = `${normalizeForMatch(track.title)} ${normalizeForMatch(track.artists[0])}`;
  const search = await withRetry(() =>
    netease.search({ keywords: query, limit: 30, type: 1 })
  );
  const songs = search.body?.result?.songs || [];

  if (songs.length === 0) {
    throw new Error('No song candidates returned by Netease search.');
  }

  const title = normalizeForMatch(track.title);
  const artist = normalizeForMatch(track.artists[0]);
  const ranked = songs
    .map((song) => {
      const songTitle = normalizeForMatch(song.name || '');
      const songArtists = normalizeForMatch(
        (song.artists || []).map((item) => item.name).join(' / ')
      );
      const exactTitle = songTitle === title;
      const exactArtist =
        songArtists === artist ||
        songArtists.split('/').map((item) => item.trim()).includes(artist);
      return {
        song,
        score: scoreCandidate(track, song),
        exactTitle,
        exactArtist,
      };
    })
    .sort((a, b) => {
      if (a.exactTitle !== b.exactTitle) return Number(b.exactTitle) - Number(a.exactTitle);
      if (a.exactArtist !== b.exactArtist) return Number(b.exactArtist) - Number(a.exactArtist);
      return b.score - a.score;
    });

  const best = ranked[0];
  if (!best || best.score < 40) {
    throw new Error('No confident lyric match found.');
  }

  const lyric = await withRetry(() => netease.lyric_new({ id: best.song.id }));
  const lrc = lyric.body?.lrc?.lyric;
  const yrc = lyric.body?.yrc?.lyric || '';
  if (!lrc) {
    throw new Error('Matched song has no lyric payload.');
  }

  const outDir = process.env.LYRICDOCK_CACHE_DIR
    ? path.resolve(process.env.LYRICDOCK_CACHE_DIR)
    : path.join(process.cwd(), 'lyrics-cache');
  await fs.mkdir(outDir, { recursive: true });

  const cacheKey = crypto
    .createHash('sha256')
    .update(
      JSON.stringify({
        title: track.title,
        artists: track.artists,
        album: track.album,
        durationMs: track.durationMs,
      })
    )
    .digest('hex')
    .slice(0, 12);

  const preferredBaseName = sanitizeFileSegment(
    `${track.artists[0] || '未知歌手'} - ${track.title}`
  );
  const baseName = await pickAvailableBaseName(outDir, preferredBaseName, cacheKey);

  const lrcPath = path.join(outDir, `${baseName}.lrc`);
  const yrcPath = path.join(outDir, `${baseName}.yrc`);
  const jsonPath = path.join(outDir, `${baseName}.json`);

  await fs.writeFile(lrcPath, lrc, 'utf8');
  if (yrc.trim()) {
    await fs.writeFile(yrcPath, yrc, 'utf8');
  } else {
    await fs.rm(yrcPath, { force: true });
  }
  await fs.writeFile(
    jsonPath,
    JSON.stringify(
      {
        fetchedAt: new Date().toISOString(),
        sourceType: 'cache',
        track,
        query,
        matched: {
          id: best.song.id,
          name: best.song.name,
          artists: (best.song.artists || []).map((item) => item.name),
          album: best.song.album?.name || '',
          durationMs: best.song.duration,
          score: best.score,
        },
        lyrics: {
          hasLrc: !!lrc,
          hasYrc: !!yrc.trim(),
        },
      },
      null,
      2
    ),
    'utf8'
  );

  console.log(
    JSON.stringify(
      {
        lrcPath,
        yrcPath: yrc.trim() ? yrcPath : null,
        jsonPath,
        track,
        matchedId: best.song.id,
        matchedTitle: best.song.name,
        matchedArtists: (best.song.artists || []).map((item) => item.name),
        score: best.score,
        hasYrc: !!yrc.trim(),
      },
      null,
      2
    )
  );
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
