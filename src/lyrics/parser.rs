use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub time_ms: u64,
    pub end_time_ms: u64,
    pub text: String,
    pub segments: Vec<LyricSegment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricSegment {
    pub start_time_ms: u64,
    pub end_time_ms: u64,
    pub text: String,
}

pub fn parse_lrc(content: &str) -> Vec<LyricLine> {
    let mut parsed = Vec::new();

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut rest = trimmed;
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else {
                break;
            };
            let tag = &stripped[..end];
            if let Some(ms) = parse_timestamp(tag) {
                times.push(ms);
                rest = &stripped[end + 1..];
            } else {
                break;
            }
        }

        let text = rest.trim();
        if times.is_empty() || text.is_empty() || is_credit_line(text) {
            continue;
        }

        for time_ms in times {
            parsed.push(LyricLine {
                time_ms,
                end_time_ms: time_ms,
                text: text.to_string(),
                segments: Vec::new(),
            });
        }
    }

    parsed.sort_by_key(|line| line.time_ms);
    for idx in 0..parsed.len() {
        let default_end = parsed[idx].time_ms.saturating_add(4_000);
        let next_start = parsed.get(idx + 1).map(|line| line.time_ms).unwrap_or(default_end);
        parsed[idx].end_time_ms = next_start.max(parsed[idx].time_ms.saturating_add(500));
    }
    parsed
}

pub fn parse_yrc(content: &str) -> Vec<LyricLine> {
    let mut parsed = Vec::new();

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('{') || !trimmed.starts_with('[') {
            continue;
        }

        let Some(header_end) = trimmed.find(']') else {
            continue;
        };
        let header = &trimmed[1..header_end];
        let mut header_parts = header.split(',');
        let Some(start_ms) = header_parts.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let Some(duration_ms) = header_parts.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };

        let mut rest = &trimmed[header_end + 1..];
        let mut segments = Vec::new();

        while let Some(segment_start_index) = rest.find('(') {
            if segment_start_index > 0 {
                rest = &rest[segment_start_index..];
            }
            let Some(segment_end_index) = rest.find(')') else {
                break;
            };
            let segment_meta = &rest[1..segment_end_index];
            let mut meta_parts = segment_meta.split(',');
            let Some(seg_start) = meta_parts.next().and_then(|value| value.parse::<u64>().ok()) else {
                break;
            };
            let Some(seg_duration_cs) = meta_parts.next().and_then(|value| value.parse::<u64>().ok()) else {
                break;
            };

            let after_meta = &rest[segment_end_index + 1..];
            let next_segment_index = after_meta.find('(').unwrap_or(after_meta.len());
            let segment_text = after_meta[..next_segment_index].to_string();

            if !segment_text.trim().is_empty() {
                // 这里的逐字段时长与下一个片段的开始时间能够直接对齐，
                // 实测这些 yrc 数据已经是毫秒单位，不需要再乘 10。
                let seg_duration_ms = seg_duration_cs;
                segments.push(LyricSegment {
                    start_time_ms: seg_start,
                    end_time_ms: seg_start.saturating_add(seg_duration_ms.max(1)),
                    text: segment_text,
                });
            }

            rest = &after_meta[next_segment_index..];
        }

        if segments.is_empty() {
            continue;
        }

        let text = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>()
            .trim()
            .to_string();
        if text.is_empty() || is_credit_line(&text) {
            continue;
        }

        parsed.push(LyricLine {
            time_ms: start_ms,
            end_time_ms: start_ms.saturating_add(duration_ms.max(1)),
            text,
            segments,
        });
    }

    parsed.sort_by_key(|line| line.time_ms);
    for idx in 0..parsed.len() {
        let default_end = parsed[idx].end_time_ms.max(parsed[idx].time_ms.saturating_add(500));
        let next_start = parsed.get(idx + 1).map(|line| line.time_ms).unwrap_or(default_end);
        parsed[idx].end_time_ms = next_start.max(parsed[idx].time_ms.saturating_add(500));
    }
    parsed
}

pub fn line_progress(line: &LyricLine, position_ms: u64) -> f32 {
    if !line.segments.is_empty() {
        let total_width: usize = line
            .segments
            .iter()
            .map(|segment| UnicodeWidthStr::width(segment.text.as_str()).max(1))
            .sum();
        if total_width == 0 {
            return 0.0;
        }

        let mut sung_width = 0.0f32;
        for segment in &line.segments {
            let segment_width = UnicodeWidthStr::width(segment.text.as_str()).max(1) as f32;
            if position_ms >= segment.end_time_ms {
                sung_width += segment_width;
                continue;
            }
            if position_ms > segment.start_time_ms {
                let duration = segment.end_time_ms.saturating_sub(segment.start_time_ms).max(1);
                let segment_progress =
                    (position_ms.saturating_sub(segment.start_time_ms)) as f32 / duration as f32;
                sung_width += segment_width * segment_progress.clamp(0.0, 1.0);
            }
            break;
        }

        return (sung_width / total_width as f32).clamp(0.0, 1.0);
    }

    if position_ms <= line.time_ms {
        return 0.0;
    }
    if position_ms >= line.end_time_ms {
        return 1.0;
    }

    let duration = line.end_time_ms.saturating_sub(line.time_ms);
    if duration == 0 {
        return 1.0;
    }
    ((position_ms - line.time_ms) as f32 / duration as f32).clamp(0.0, 1.0)
}

fn parse_timestamp(tag: &str) -> Option<u64> {
    let mut parts = tag.split(':');
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds_part = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let mut seconds_split = seconds_part.split('.');
    let seconds = seconds_split.next()?.parse::<u64>().ok()?;
    let millis_text = seconds_split.next().unwrap_or("0");
    let millis = match millis_text.len() {
        0 => 0,
        1 => millis_text.parse::<u64>().ok()? * 100,
        2 => millis_text.parse::<u64>().ok()? * 10,
        _ => millis_text.get(..3)?.parse::<u64>().ok()?,
    };

    Some(minutes * 60_000 + seconds * 1_000 + millis)
}

fn is_credit_line(text: &str) -> bool {
    ["作词", "作曲", "编曲", "制作人", "监制"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::{line_progress, parse_lrc, parse_yrc};

    #[test]
    fn parse_lrc_builds_timed_lines() {
        let lines = parse_lrc("[00:01.00]第一句\n[00:03.50]第二句\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "第一句");
        assert_eq!(lines[0].time_ms, 1000);
        assert_eq!(lines[0].end_time_ms, 3500);
    }

    #[test]
    fn parse_yrc_builds_segments_and_skips_metadata() {
        let content = "{\"t\":0,\"c\":[{\"tx\":\"作词: \"}]}\n[1000,2000](1000,50,0)你(1500,50,0)好";
        let lines = parse_yrc(content);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "你好");
        assert_eq!(lines[0].segments.len(), 2);
        assert_eq!(lines[0].segments[0].start_time_ms, 1000);
        assert_eq!(lines[0].segments[0].end_time_ms, 1050);
        assert_eq!(lines[0].segments[1].text, "好");
    }

    #[test]
    fn line_progress_prefers_segment_timeline() {
        let lines = parse_yrc("[1000,2000](1000,50,0)你(1500,50,0)好");
        let line = &lines[0];
        assert_eq!(line_progress(line, 900), 0.0);
        assert!(line_progress(line, 1250) > 0.0);
        assert!(line_progress(line, 1600) > 0.5);
        assert_eq!(line_progress(line, 2100), 1.0);
    }
}
