use std::{
    fmt::Debug,
    io::{Read, Write},
};

use anyhow::Context;
use clap::Parser;
use serde_bytes::ByteBuf;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DataFile<H, D> {
    headers: H,
    body: D,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BackupDataV1<S, H, D> {
    prefix: S,
    info: DataFile<H, D>,
    data_zip: DataFile<H, D>,
    suffix: S,
}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
enum BackupData<S, H, D> {
    V1(BackupDataV1<S, H, D>),
}

fn decompress_zip(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut reader =
        zip::ZipArchive::new(std::io::Cursor::new(data)).context("failed to open archive")?;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

    if !reader.comment().is_empty() {
        writer
            .set_raw_comment(reader.comment().to_vec().into_boxed_slice())
            .context("failed to copy comment")?;
    }
    if let Some(d) = reader.raw_zip64_extensible_data_sector() {
        writer.set_raw_zip64_extensible_data_sector(d.to_vec().into_boxed_slice());
    }

    for idx in 0..reader.len() {
        reader
            .by_index(idx)
            .context("failed to get file")
            .and_then(|mut f| {
                let mut opt = f
                    .options()
                    .compression_method(zip::CompressionMethod::Stored)
                    .into_full_options();
                if !f.comment().is_empty() {
                    opt = opt.with_file_comment(f.comment());
                }
                if let Some(d) = f.extra_data() {
                    let mut d = d.as_slice();
                    while !d.is_empty() {
                        let (id, data, tail) = d
                            .split_first_chunk()
                            .and_then(|(id, tail)| {
                                let (sz, tail) = tail.split_first_chunk()?;
                                let id = u16::from_le_bytes(*id);
                                let size = u16::from_le_bytes(*sz);
                                let (data, tail) =
                                    tail.split_at_checked(size as usize).unwrap_or((tail, b""));
                                Some((id, data, tail))
                            })
                            .context("failed to decode extra data")?;
                        opt.add_extra_field(id, data, false)
                            .context("failed to add extra data")?;
                        d = tail;
                    }
                }
                if f.is_file() {
                    writer
                        .start_file(f.name()?, opt)
                        .context("failed to start file")?;
                    let _ = std::io::copy(&mut f, &mut writer).context("failed to copy content")?;
                    Ok(())
                } else if f.is_dir() {
                    writer
                        .add_directory(f.name()?, opt)
                        .context("failed to add dir")
                } else {
                    todo!()
                }
            })
            .with_context(|| format!("failed to decompress file {idx}"))?;
    }

    writer
        .finish()
        .context("failed to finish zip")
        .map(std::io::Cursor::into_inner)
}

fn parse_data(
    decompress: bool,
    input: &str,
) -> anyhow::Result<
    BackupDataV1<&str, Vec<(&'static str, serde_bytes::ByteBuf)>, serde_bytes::ByteBuf>,
> {
    anyhow::ensure!(
        input.starts_with("<!DOCTYPE html>\n<!-- Version: 1 -->"),
        "unknown version"
    );
    const MIME_START: &str = "<!-- Begin inline MIME --\n";
    const MIME_END: &str = "\n---- End inline MIME -->";

    let (prefix, mime, suffix) = input
        .find(MIME_START)
        .and_then(|start| {
            let (prefix, tail) = input.split_at_checked(start + MIME_START.len())?;
            let end = tail.rfind(MIME_END)?;
            let (mime, suffix) = tail.split_at_checked(end)?;
            Some((prefix, mime, suffix))
        })
        .context("can't find inline mime")?;

    let mime = mailparse::parse_mail(mime.as_bytes()).context("failed to parse mime")?;
    anyhow::ensure!(
        mime.ctype.mimetype == "multipart/mixed",
        "unknown mime content type"
    );
    anyhow::ensure!(
        mime.headers.len() == 1,
        "mime contains unknown header:\n{:#?}",
        mime.headers
    );

    let mut info = None;
    let mut data = None;
    for sub in mime.subparts {
        let disp = sub.get_content_disposition();
        anyhow::ensure!(
            disp.disposition == mailparse::DispositionType::Attachment,
            "unknown disposition type"
        );
        let headers = {
            let mut ret = Vec::with_capacity(sub.headers.len());
            for h in sub.get_headers() {
                let k = h.get_key_ref();
                if k.eq_ignore_ascii_case("content-length")
                    || k.eq_ignore_ascii_case("content-transfer-encoding")
                {
                    continue;
                }
                let k = if k.eq_ignore_ascii_case("content-type") {
                    "Content-Type"
                } else if k.eq_ignore_ascii_case("content-disposition") {
                    "Content-Disposition"
                } else {
                    anyhow::bail!("unknown header: {k}");
                };
                ret.push((k, serde_bytes::Bytes::new(h.get_value_raw()).to_owned()));
            }
            ret
        };
        let body = serde_bytes::ByteBuf::from(sub.get_body_raw().context("failed to decode body")?);
        match disp
            .params
            .get("filename")
            .context("missing filename")?
            .as_str()
        {
            "archive.json" => match &mut info {
                Some(_) => return Err(anyhow::Error::msg("duplicated archive.json")),
                None => {
                    info = Some(DataFile { headers, body });
                }
            },
            "archive.zip" => match &mut data {
                Some(_) => return Err(anyhow::Error::msg("duplicated archive.zip")),
                None => {
                    data = Some(DataFile { headers, body });
                }
            },
            f => anyhow::bail!("Unknown file {f:?}"),
        }
    }
    let info = info.context("missing archive.json")?;
    let data_zip = {
        let d = data.context("missing archive.zip")?;
        if decompress {
            DataFile {
                headers: d.headers,
                body: decompress_zip(&d.body)
                    .context("failed to decompress zip data")?
                    .into(),
            }
        } else {
            d
        }
    };

    Ok(BackupDataV1 {
        prefix,
        info,
        data_zip,
        suffix,
    })
}

#[derive(clap::Subcommand)]
enum Cmd {
    ToCbor {
        #[arg(long)]
        no_decompress: bool,
        input: String,
        output: String,
    },
    TarToCbor {
        input: Option<String>,
        output: Option<String>,
    },
    ToHtml {
        input: String,
        output: String,
    },
}

#[derive(clap::Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

fn to_cbor(decompress: bool, input: &str, output: &str) -> anyhow::Result<()> {
    let data = std::fs::read_to_string(input).context("failed to read input file")?;
    let backup = parse_data(decompress, &data).context("failed to parse data")?;
    let mut buf = Vec::new();
    ciborium::into_writer(&BackupData::V1(backup), &mut buf).unwrap();
    std::fs::write(output, &buf).context("failed to write output")
}

fn encode_data(
    buf: &mut Vec<u8>,
    d: &BackupDataV1<String, Vec<(String, serde_bytes::ByteBuf)>, ByteBuf>,
) {
    macro_rules! boundary_txt {
        () => {
            "----=_Part_boundary"
        };
    }
    macro_rules! boundary_full {
        () => {
            concat!("--", boundary_txt!())
        };
    }
    macro_rules! boundary_n {
        () => {
            concat!(boundary_full!(), "\n")
        };
    }
    buf.extend_from_slice(d.prefix.as_bytes());
    buf.extend_from_slice(
        concat!(
            concat!(
                "Content-Type: multipart/mixed; boundary=\"",
                boundary_txt!(),
                "\"\n"
            ),
            "\n",
            boundary_n!(),
        )
        .as_bytes(),
    );
    for (k, v) in d.info.headers.iter() {
        buf.extend_from_slice(k.as_bytes());
        buf.extend_from_slice(b": ");
        buf.extend_from_slice(v.as_slice());
        buf.extend_from_slice(b"\n");
    }
    let _ = writeln!(&mut *buf, "Content-Length: {}\n", d.info.body.len());
    buf.extend_from_slice(d.info.body.as_slice());

    buf.extend_from_slice(
        concat!("\n", boundary_n!(), "Content-Transfer-Encoding: base64\n",).as_bytes(),
    );
    let base64_enc = {
        let mut spec = data_encoding::BASE64_MIME.specification();
        spec.wrap.width = 0;
        spec.encoding().unwrap()
    };
    let zip_base64 = base64_enc.encode(d.data_zip.body.as_slice());

    for (k, v) in d.data_zip.headers.iter() {
        buf.extend_from_slice(k.as_bytes());
        buf.extend_from_slice(b": ");
        buf.extend_from_slice(v.as_slice());
        buf.extend_from_slice(b"\n");
    }
    writeln!(&mut *buf, "Content-Length: {}\n", zip_base64.len()).unwrap();
    buf.extend_from_slice(zip_base64.as_bytes());

    buf.extend_from_slice(concat!("\n", boundary_full!()).as_bytes());
    buf.extend_from_slice(d.suffix.as_bytes());
}

fn to_html(input: &str, output: &str) -> anyhow::Result<()> {
    let info = std::fs::read(input).context("failed to read input file")?;
    let data = ciborium::from_reader(info.as_slice()).context("failed to decode cbor")?;
    let mut buf = Vec::new();
    encode_data(
        &mut buf,
        match &data {
            BackupData::V1(d) => d,
        },
    );
    std::fs::write(output, &buf).context("failed to write output")
}

fn tar_to_cbor(
    input: &mut dyn std::io::Read,
    output: &mut dyn std::io::Write,
) -> anyhow::Result<()> {
    let mut input = tar::Archive::new(std::io::BufReader::new(input));
    let mut output = tar::Builder::new(std::io::BufWriter::new(output));
    let mut in_buf = Vec::new();
    let mut out_buf = Vec::new();
    let mut path_buf = std::path::PathBuf::new();

    for e in input.entries()? {
        let mut e = e?;

        path_buf.clear();
        {
            let p = e.path()?;
            path_buf.push(p.as_os_str());
            eprintln!("{}", p.display());
        }

        let mut h = e.header().clone();
        match h.entry_type() {
            tar::EntryType::Regular => {
                in_buf.clear();
                e.read_to_end(&mut in_buf)?;
                let d = parse_data(
                    true,
                    std::str::from_utf8(&in_buf).context("invalid utf8 in html")?,
                )
                .context("failed to parse data")?;

                out_buf.clear();
                ciborium::into_writer(&BackupData::V1(d), &mut out_buf)?;

                h.set_size(out_buf.len() as u64);
                path_buf.set_extension("bin");
                output.append_data(&mut h, &path_buf, out_buf.as_slice())?;
            }
            tar::EntryType::Directory => {
                output.append_data(&mut h, &path_buf, std::io::empty())?;
            }
            tar::EntryType::Link | tar::EntryType::Symlink => {
                output.append_link(
                    &mut h,
                    &path_buf,
                    e.link_name()?.context("missing link name for link")?,
                )?;
            }
            tar::EntryType::GNULongLink | tar::EntryType::GNULongName => continue,
            tar::EntryType::XHeader => {
                in_buf.clear();
                e.read_to_end(&mut in_buf)?;
                output.append(&h, in_buf.as_slice())?;
            }
            t => anyhow::bail!("unknown header type: {t:?}"),
        }
    }

    output
        .into_inner()?
        .into_inner()
        .map_err(std::io::IntoInnerError::into_error)?;
    Ok(())
}
enum MaybeFileIo<T> {
    File(std::fs::File),
    Other(T),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::ToCbor {
            no_decompress,
            input,
            output,
        } => to_cbor(!no_decompress, &input, &output),
        Cmd::TarToCbor { input, output } => {
            let mut input = match input {
                Some(i) => MaybeFileIo::File(
                    std::fs::File::options()
                        .read(true)
                        .open(&i)
                        .context("failed to open input")?,
                ),
                None => MaybeFileIo::Other(std::io::stdin().lock()),
            };
            let mut output = match output {
                Some(o) => MaybeFileIo::File(
                    std::fs::File::options()
                        .create_new(true)
                        .write(true)
                        .open(&o)
                        .context("failed to open output")?,
                ),
                None => MaybeFileIo::Other(std::io::stdout().lock()),
            };
            tar_to_cbor(
                match &mut input {
                    MaybeFileIo::File(f) => &mut *f,
                    MaybeFileIo::Other(o) => &mut *o,
                },
                match &mut output {
                    MaybeFileIo::File(f) => &mut *f,
                    MaybeFileIo::Other(o) => &mut *o,
                },
            )
        }
        Cmd::ToHtml { input, output } => to_html(&input, &output),
    }
}
