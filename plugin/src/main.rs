use age_core::format::{FileKey, Stanza, AgeStanza};
use age_core::secrecy::ExposeSecret;
use cookie_factory::{WriteContext, GenResult};
use nom_bufreader::bufreader::BufReader;
use nom_bufreader::Parse;
use cookie_factory::combinator::slice;

use age::Identity;
use age_core::format;
use age::cli_common::UiCallbacks;
use clap::{Parser,ArgAction::SetTrue, arg, command, Command};
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::str::FromStr;
use std::string::String;
use std::sync::mpsc::{Receiver, Sender};

use age_plugin_threshold::crypto::{self, SecretShare};
use rlp::{Decodable, Encodable, RlpDecodable, RlpEncodable, RlpStream};

use age_plugin_threshold::types::GenericIdentity;
use age_plugin_threshold::types::GenericRecipient;
use age_plugin_threshold::types::ThresholdIdentity;
use age_plugin_threshold::types::ThresholdRecipient;


fn read_text_file(path: &str) -> io::Result<Vec::<String>> {
use std::io::BufReader;
    let file = File::open(path)?;
    let mut v = vec![];
    for l in BufReader::new(file).lines() {
        let line = l?;
            if !line.starts_with("#") {
                v.push(line.trim().to_string());
            }
    }
    Ok(v)
}

#[derive(clap::Parser)]
#[command(name = "three")]
#[command(bin_name = "three")]
struct Cli {
    #[clap(short, long)]
    encrypt: bool,
    #[clap(short, long)]
    decrypt: bool,
    #[clap(short, long)]
    threshold: Option<usize>,
    #[clap(short, long)]
    recipient: Vec<String>,
    #[clap(short, long)]
    identity: Vec<String>,
}

use nom::bytes::streaming::tag;

fn main() -> io::Result<()> {
    /* AGE:
     * Usage:
     *    age [--encrypt] (-r RECIPIENT | -R PATH)... [--armor] [-o OUTPUT] [INPUT]
     *    age [--encrypt] --passphrase [--armor] [-o OUTPUT] [INPUT]
     *    age --decrypt [-i PATH]... [-o OUTPUT] [INPUT]
     *
     * Options:
     *    -e, --encrypt               Encrypt the input to the output. Default if omitted.
     *    -d, --decrypt               Decrypt the input to the output.
     *    -o, --output OUTPUT         Write the result to the file at path OUTPUT.
     *    -a, --armor                 Encrypt to a PEM encoded format.
     *    -p, --passphrase            Encrypt with a passphrase.
     *    -r, --recipient RECIPIENT   Encrypt to the specified RECIPIENT. Can be repeated.
     *    -R, --recipients-file PATH  Encrypt to recipients listed at PATH. Can be repeated.
     *    -i, --identity PATH         Use the identity file at PATH. Can be repeated.
     */
    let cmd = Cli::parse();

    let mut recipients = vec![];
    for r in cmd.recipient {
                recipients.push(GenericRecipient::from_bech32(r.as_str()).map_err(io::Error::other)?);
    }

    let mut identities = vec![];
    for id in cmd.identity {
        let lines = read_text_file(&id)?;
                identities.push(GenericIdentity::from_bech32(&lines[0]).map_err(io::Error::other)?);
    }

    if cmd.encrypt && cmd.decrypt {
        return Err(io::Error::other("cannot encrypt and decrypt at the same time"));
    }

    if !cmd.decrypt {
    let threshold = cmd.threshold.unwrap_or(recipients.len()/2+1);

    if recipients.len() < threshold {
        return Err(io::Error::other("not enough recipients"));
    }

    let callbacks = UiCallbacks{};
    let secret = FileKey::from([9; 16]);
    let shares = crypto::share_secret(&secret, threshold, recipients.len());
    let mut shares_stanzas = vec![];
    for (r,s) in recipients.iter().zip(shares.iter()) {
        let recipient = r.to_recipient(callbacks).map_err(io::Error::other)?;
        let mut r = recipient.wrap_file_key(&s.file_key).map_err(io::Error::other)?;
        if r.len() != 1 {
            return Err(io::Error::other("encryption produced multiple stanzas"));
        }
        let stanza = r.remove(0);
        shares_stanzas.push(stanza);
    }
    let mut wc = cookie_factory::WriteContext{write: std::io::stdout(), position: 0};
    wc = serialize(threshold, &shares_stanzas)(wc).map_err(io::Error::other)?;
    } else {
    let callbacks = UiCallbacks{};
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin);
        let prelude = reader.parse(deserialize).map_err(|err| match err {
            nom_bufreader::Error::Error(err) => io::Error::new(io::ErrorKind::InvalidData, "parse error"),
            nom_bufreader::Error::Failure(err) => io::Error::new(io::ErrorKind::InvalidData, "parse error"),
            nom_bufreader::Error::Io(err) => err,
            nom_bufreader::Error::Eof => io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected eof"),
        })?;
        dbg!(&prelude);
        let mut shares = vec![];
        for (i,s) in prelude.shares_stanzas.iter().enumerate() {
            if shares.len() >= prelude.threshold {
                break;
            }
            for identity in &identities {
                match identity.to_identity(callbacks).map_err(io::Error::other)?.unwrap_stanza(&s) {
                    Some(Ok(file_key)) => {
                        let share = SecretShare{file_key, index: (i+1).try_into().unwrap()};
                        shares.push(share);
                        break;
                    },
                    Some(Err(err)) => return Err(io::Error::other(err)),
                    None => continue,
                }
            }
        }
        dbg!(shares.len());
        if shares.len() < prelude.threshold {
            return Err(io::Error::other("not enough shares"));
        }
        let secret = crypto::reconstruct_secret(&shares);
        dbg!(secret.expose_secret());
    }

    Ok(())
}

#[derive(Debug)]
struct Prelude {
    threshold: usize,
    shares_stanzas: Vec<Stanza>,
}

fn serialize<W: Write>(t: usize, shares_stanzas: &Vec<Stanza>) -> impl Fn(WriteContext<W>) -> GenResult<W> + '_ { move |mut wc| {
    wc = format::write::age_stanza("threshold", &[&t.to_string()], &[])(wc)?;
        for s in shares_stanzas {
            wc = format::write::age_stanza(&s.tag, &s.args, &s.body)(wc)?;
        }
    wc = slice(&b"---")(wc)?;
    Ok(wc)
    }
}

fn deserialize<'a>(input: &[u8]) -> nom::IResult<&[u8], Prelude, nom::error::Error<Vec<u8>>> {
    match deserialize2(input) {
        Ok((input, prelude)) => Ok((input, prelude)),
        Err(err) => Err(err.to_owned())
    }
}
fn deserialize2<'a>(input: &[u8]) -> nom::IResult<&[u8], Prelude, nom::error::Error<&[u8]>> {
    let (input, stanza) = format::read::age_stanza(input)?;
    if stanza.tag != "threshold" {
        return Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }
    let threshold = stanza.args[0].parse::<usize>().map_err(|err| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Satisfy)))?;
    let (input, (mut stanzas,_)) = nom::multi::many_till(format::read::age_stanza, tag("---"))(input)?;
    /*
    loop {
        match format::read::age_stanza(input) {
            Ok((input_, stanza)) => {
                input = input_;
                dbg!(stanzas.len());
                dbg!(input.len());
                stanzas.push(stanza.into());
            },
            Err(nom::Err::Incomplete(needed)) => return Err(nom::Err::Incomplete(needed)),
            Err(err) => { dbg!(err); break; }
        };
    }
    */
    Ok((input, Prelude{threshold, shares_stanzas: stanzas.drain(..).map(|s| s.into()).collect()}))
}
