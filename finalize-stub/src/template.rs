//! Versioned stub capabilities and placeholder layout, independent of binary format.
const CAPS: &[u8] = b"@@RUNFILES_CAPS@@";
const CONTROL_SIZE: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub max_args: usize,
    pub arg_size: usize,
    pub flag_bits: usize,
}

impl Capabilities {
    fn read(data: &[u8]) -> Result<Self, String> {
        let Some(pos) = find(data, CAPS) else {
            // Templates released before capability metadata was introduced.
            return Ok(Self {
                max_args: 10,
                arg_size: 256,
                flag_bits: 32,
            });
        };
        let record = data
            .get(pos..pos + 64)
            .ok_or("Truncated capabilities record")?;
        if find(&data[pos + CAPS.len()..], CAPS).is_some() {
            return Err("Multiple capabilities records".into());
        }
        let end = record
            .iter()
            .position(|&b| b == 0)
            .ok_or("Unterminated capabilities record")?;
        if record[end..].iter().any(|&b| b != 0) {
            return Err("Invalid capabilities padding".into());
        }
        let text = std::str::from_utf8(&record[CAPS.len()..end])
            .map_err(|_| "Capabilities must be ASCII")?;
        let fields: Vec<_> = text.split(';').collect();
        if fields.len() != 4 || fields[0] != "v1" {
            return Err(format!("Unsupported capabilities: {text}"));
        }
        let number = |field: &str, prefix: &str| -> Result<usize, String> {
            field
                .strip_prefix(prefix)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("Invalid capability: {field}"))
        };
        let caps = Self {
            max_args: number(fields[1], "args=")?,
            arg_size: number(fields[2], "size=")?,
            flag_bits: number(fields[3], "flags=")?,
        };
        if caps.max_args == 0
            || caps.arg_size == 0
            || caps.max_args > caps.flag_bits
            || !matches!(caps.flag_bits, 32 | 64)
        {
            return Err(format!("Unsupported capabilities: {text}"));
        }
        Ok(caps)
    }

    pub fn supports(self, argv: &[String], flags: u64) -> bool {
        argv.len() <= self.max_args
            && argv.iter().all(|a| a.len() <= self.arg_size)
            && (self.flag_bits == 64 || flags >> self.flag_bits == 0)
    }
}

#[derive(Debug)]
pub struct Template {
    pub data: Vec<u8>,
    pub capabilities: Capabilities,
    argc: usize,
    flags: usize,
    export_env: usize,
    args: usize,
}

fn find(data: &[u8], pattern: &[u8]) -> Option<usize> {
    data.windows(pattern.len()).position(|w| w == pattern)
}

fn control(data: &[u8], marker: &[u8]) -> Result<usize, String> {
    let pos = find(data, marker)
        .ok_or_else(|| format!("{} placeholder not found", String::from_utf8_lossy(marker)))?;
    let region = data
        .get(pos..pos + CONTROL_SIZE)
        .ok_or("Truncated control placeholder")?;
    if region[marker.len()..].iter().any(|&b| b != 0) {
        return Err("Invalid control placeholder padding".into());
    }
    Ok(pos)
}

impl Template {
    pub fn parse(data: Vec<u8>) -> Result<Self, String> {
        let capabilities = Capabilities::read(&data)?;
        let argc = control(&data, b"@@RUNFILES_ARGC@@")?;
        let flags = control(&data, b"@@RUNFILES_TRANSFORM_FLAGS@@")?;
        let export_env = control(&data, b"@@RUNFILES_EXPORT_ENV@@")?;
        let size = capabilities
            .max_args
            .checked_mul(capabilities.arg_size)
            .ok_or("Argument storage size overflow")?;
        // Scan in linear time without allocating based on untrusted metadata.
        let mut run = 0;
        let mut args = None;
        for (i, &byte) in data.iter().enumerate() {
            run = if byte == b'@' { run + 1 } else { 0 };
            if run == size {
                args = Some(i + 1 - size);
                break;
            }
        }
        let args = args.ok_or("Argument storage does not match capabilities")?;
        let mut reserved = vec![
            (argc, CONTROL_SIZE),
            (flags, CONTROL_SIZE),
            (export_env, CONTROL_SIZE),
        ];
        if let Some(pos) = find(&data, CAPS) {
            reserved.push((pos, 64));
        }
        if reserved
            .iter()
            .any(|&(pos, len)| args < pos + len && pos < args + size)
        {
            return Err("Argument storage overlaps metadata".into());
        }
        Ok(Self {
            data,
            capabilities,
            argc,
            flags,
            export_env,
            args,
        })
    }

    pub fn patch(
        mut self,
        argv: &[String],
        flags: u64,
        export_env: bool,
    ) -> Result<Vec<u8>, String> {
        if argv.is_empty() || !self.capabilities.supports(argv, flags) {
            return Err("Arguments exceed template capabilities".into());
        }
        if argv.iter().any(|a| a.as_bytes().contains(&0)) {
            return Err("Arguments cannot contain NUL bytes".into());
        }
        let mut replace = |pos: usize, size: usize, value: &[u8]| {
            self.data[pos..pos + size].fill(0);
            self.data[pos..pos + value.len()].copy_from_slice(value);
        };
        replace(self.argc, CONTROL_SIZE, argv.len().to_string().as_bytes());
        replace(self.flags, CONTROL_SIZE, flags.to_string().as_bytes());
        replace(
            self.export_env,
            CONTROL_SIZE,
            if export_env { b"1" } else { b"0" },
        );
        for (i, arg) in argv.iter().enumerate() {
            replace(
                self.args + i * self.capabilities.arg_size,
                self.capabilities.arg_size,
                arg.as_bytes(),
            );
        }
        Ok(self.data)
    }
}

/// Ties retain command-line order; capacity alone does not determine file size.
pub fn prefer(
    candidate: &Template,
    current: Option<&Template>,
    argv: &[String],
    flags: u64,
) -> bool {
    candidate.capabilities.supports(argv, flags)
        && current.map_or(true, |old| candidate.data.len() < old.data.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(count: usize, size: usize, bits: usize, metadata: bool) -> Vec<u8> {
        let mut data = Vec::new();
        for marker in [
            b"@@RUNFILES_ARGC@@".as_slice(),
            b"@@RUNFILES_TRANSFORM_FLAGS@@",
            b"@@RUNFILES_EXPORT_ENV@@",
        ] {
            let start = data.len();
            data.extend(marker);
            data.resize(start + CONTROL_SIZE, 0);
        }
        data.extend(vec![b'@'; count * size]);
        if metadata {
            let start = data.len();
            data.extend(
                format!("@@RUNFILES_CAPS@@v1;args={count};size={size};flags={bits}").bytes(),
            );
            data.resize(start + 64, 0);
        }
        data
    }

    #[test]
    fn exact_boundaries_and_utf8_bytes() {
        for (count, size, bits) in [(10, 256, 32), (40, 4096, 64)] {
            let t = Template::parse(fixture(count, size, bits, true)).unwrap();
            let argv = vec!["é".repeat(size / 2); count];
            assert!(t.capabilities.supports(&argv, 1 << (count - 1)));
            assert!(!t.capabilities.supports(&vec!["a".into(); count + 1], 0));
            assert!(!t.capabilities.supports(&["x".repeat(size + 1)], 0));
            let base = t.args;
            let flags_pos = t.flags;
            let patched = t.patch(&argv, 1 << (count - 1), false).unwrap();
            assert_eq!(&patched[base..base + size], argv[0].as_bytes());
            assert_eq!(
                &patched[base + (count - 1) * size..base + count * size],
                argv[count - 1].as_bytes()
            );
            assert!(patched[flags_pos..].starts_with((1u64 << (count - 1)).to_string().as_bytes()));
        }
    }

    #[test]
    fn legacy_and_selection_in_either_order() {
        let small = Template::parse(fixture(10, 256, 32, false)).unwrap();
        let large = Template::parse(fixture(40, 4096, 64, true)).unwrap();
        let short = vec!["echo".into()];
        assert!(prefer(&small, Some(&large), &short, 0));
        assert!(!prefer(&large, Some(&small), &short, 0));
        assert!(prefer(&large, None, &vec!["a".into(); 11], 0));
        assert!(!prefer(&small, None, &vec!["a".into(); 11], 0));
        assert!(!prefer(&small, None, &["a".repeat(257)], 0));
        assert!(prefer(&large, None, &["a".repeat(257)], 0));
        assert!(!small.capabilities.supports(&short, 1 << 39));
        // Choose actual bytes, even if a smaller-capacity file has more padding.
        let mut padded = Template::parse(fixture(10, 256, 32, true)).unwrap();
        padded.data.resize(large.data.len() + 1, 0);
        assert!(prefer(&large, Some(&padded), &short, 0));
    }

    #[test]
    fn malformed_and_truncated_templates() {
        assert!(Template::parse(vec![]).is_err());
        let data = fixture(40, 4096, 64, true);
        assert!(Template::parse(data[..data.len() - 1].to_vec()).is_err());
        let mut short = data.clone();
        short.drain(96..97);
        assert!(Template::parse(short).is_err());
        assert!(Template::parse(fixture(40, 4096, 32, true)).is_err());
        let mut unknown = data.clone();
        let pos = find(&unknown, b"v1;").unwrap();
        unknown[pos + 1] = b'2';
        assert!(Template::parse(unknown).is_err());
        let mut duplicate = data.clone();
        duplicate.extend_from_slice(&data[data.len() - 64..]);
        assert!(Template::parse(duplicate).is_err());
    }
}
