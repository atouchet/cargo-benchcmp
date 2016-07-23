extern crate rustc_serialize;
extern crate docopt;
#[macro_use]
extern crate lazy_static;
extern crate regex;
#[macro_use]
extern crate prettytable;

mod benchmark;
mod utils;
mod error;

use docopt::Docopt;
use regex::Regex;
use prettytable::Table;
use prettytable::format;

use benchmark::{Benchmark, parse_benchmarks};
use utils::find_overlap;
use error::Result;

use std::io;
use std::io::prelude::*;
use std::fs::File;

macro_rules! err_println {
    ($fmt:expr) => (err_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => (err_print!(concat!($fmt, "\n"), $($arg)*));
}

macro_rules! err_print {
    ($($arg:tt)*) => (io::stderr().write_fmt(format_args!($($arg)*)).unwrap(););
}

const USAGE: &'static str = r#"
Compares Rust micro-benchmark results.

Usage:
    cargo benchcmp [options] <file> <file>
    cargo benchcmp [options] <name> <name> <file>...
    cargo benchcmp -h | --help

The first version takes two file and compares the common bench-tests.
The second version takes two module names and one or more files, and compares
the common bench-tests of the two modules.

Options:
    -h, --help           show this help message and exit
    --threshold <n>      only show comparisons with a percentage change greater
                         than this threshold
    --variance           show variance
    --show <option>      show regressions, improvements or both [default: both]
    --strip-fst <regex>  a regex to strip from first benchmarks' names
    --strip-snd <regex>  a regex to strip from second benchmarks' names
"#;

#[derive(Debug, RustcDecodable)]
struct Args {
    flag_threshold: Option<u8>,
    flag_variance: bool,
    flag_show: ShowOption,
    flag_strip_fst: Option<String>,
    flag_strip_snd: Option<String>,
    arg_name: Option<[String; 2]>,
    arg_file: Vec<String>,
}

#[derive(Debug, RustcDecodable, PartialEq, Eq)]
enum ShowOption {
    Regressions,
    Improvements,
    Both,
}

fn main() {
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.decode())
        .unwrap_or_else(|e| e.exit());


    let pairs = match read_benchmarks(&args) {
        Err(e) => {
            err_println!("{}", e);
            return;
        }
        Ok(pairs) => pairs,
    };

    write_pairs(args, pairs);
}

/// Write the pairs of benchmarks in a table, along with their comparison
fn write_pairs(args: Args, pairs: Vec<(Benchmark, Benchmark)>) {
    use ShowOption::{Regressions, Improvements};

    let names = args.arg_name.map_or(args.arg_file, |a| a.to_vec());

    let mut output = Table::new();
    output.set_format(*format::consts::FORMAT_CLEAN);

    output.add_row(row![
        d->"name",
        format!("{} ns/iter", names[0]),
        format!("{} ns/iter", names[1]),
        r->"diff ns/iter",
        r->"diff %"]);

    for comparison in pairs.into_iter().map(|(f, s)| f.compare(s)) {
        let trunc_abs_per = (comparison.diff_ratio * 100f64).abs().trunc() as u8;

        if args.flag_threshold.map_or(false, |threshold| trunc_abs_per < threshold) ||
           args.flag_show == Regressions && comparison.diff_ns <= 0 ||
           args.flag_show == Improvements && comparison.diff_ns >= 0 {
            continue;
        }

        output.add_row(comparison.to_row(args.flag_variance));
    }

    output.printstd();
}

/// Read the benchmarks,
///  filter by module name,
///  do the regex replace,
///  and find the benchmarks that overlap.
fn read_benchmarks(args: &Args) -> Result<Vec<(Benchmark, Benchmark)>> {
    let files: std::result::Result<Vec<File>, io::Error> =
        args.arg_file.iter().map(File::open).collect();

    let files: Vec<File> = try!(files);

    let (fst, snd) = match args.arg_name {
        None => {
            let mut benches_iter = files.into_iter().map(parse_benchmarks);
            let fst = benches_iter.next().unwrap();
            let snd = benches_iter.next().unwrap();

            (fst, snd)
        }
        Some(ref names) => {
            let benchmarks = files.into_iter().flat_map(parse_benchmarks);

            let mut fst = Vec::new();
            let mut snd = Vec::new();

            for mut b in benchmarks {
                let name = b.name;
                let mut split = name.splitn(2, "::");
                // There should always be a string here
                let implementation = split.next().unwrap();
                // But there may not be a :: in the string, so the second part may not exist
                if let Some(test) = split.next() {
                    b.name = String::from(test);
                    if implementation == &names[0] {
                        fst.push(b);
                    } else if implementation == &names[1] {
                        snd.push(b);
                    }
                }
            }

            (Box::new(fst.into_iter()) as Box<Iterator<Item=Benchmark>>, Box::new(snd.into_iter()) as Box<Iterator<Item=Benchmark>>)
        }
    };

    let mut fst: Vec<Benchmark> = try!(strip_names(fst, &args.flag_strip_fst));
    let mut snd: Vec<Benchmark> = try!(strip_names(snd, &args.flag_strip_snd));

    fst.sort_by(|b1, b2| b1.name.cmp(&b2.name));
    snd.sort_by(|b1, b2| b1.name.cmp(&b2.name));

    let overlap = find_overlap(fst, snd, |b1, b2| b1.name.cmp(&b2.name));

    warn_missing(overlap.left,
                 "WARNING: benchmarks present in fst but not in snd");
    warn_missing(overlap.right,
                 "WARNING: benchmarks present in snd but not in fst");

    Ok(overlap.overlap)
}

/// Filter the names in every benchmark, based on the regex string
fn strip_names<I: Iterator<Item=Benchmark>>(benches: I,
               strip: &Option<String>)
               -> Result<Vec<Benchmark>> {
    match *strip {
        None => Ok(benches.collect()),
        Some(ref s) => {
            let re = try!(Regex::new(s.as_str()));
            Ok(benches.map(|mut b| {
                    b.filter_name(&re);
                    b
                })
                .collect())
        }
    }
}

/// Print a warning message if there are benchmarks outside of the overlap
fn warn_missing(v: Vec<Benchmark>, s: &str) {
    use std::ops::Not;

    if v.is_empty().not() {
        err_println!("{}: {:?}",
                     s,
                     v.into_iter()
                         .map(|n| n.name)
                         .collect::<Vec<String>>());
    }
}