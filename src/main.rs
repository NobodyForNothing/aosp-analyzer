use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Write};
use std::path::{PathBuf};
use std::time::SystemTime;
use peak_alloc::PeakAlloc;
use rayon::prelude::IntoParallelRefIterator;
use rayon::iter::{Either, ParallelIterator};
use crate::counter::{CountContext, HasStats, SourceFile};
use crate::language::Language;

mod counter;
mod language;
mod lang_stats;
mod file_stats;

#[cfg(debug_assertions)]
#[global_allocator]
static PEAK_ALLOC: PeakAlloc = PeakAlloc;

const ANDROID_SOURCE: &str = "/home/derdilla/android-source/aosp14/";
//const ANDROID_SOURCE: &str = "/home/derdilla/Coding/Java";

fn main() {
    /*
    let scan = Scanner::scan(ANDROID_SOURCE);
    let mut context = CountContext::new();
    for f in scan.files {
        context.insert_file(f.unwrap());
    }

    println!("{:#?}", context.stats());

     */
    let start = SystemTime::now();

    let context = scan_dir(PathBuf::from(ANDROID_SOURCE));
    //println!("{:#?}", context);
    println!("Analyzing {ANDROID_SOURCE} took: {}ms", SystemTime::now().duration_since(start).unwrap().as_millis());
    // 511ms -> 941ms -> 524ms -> 509ms -> 477ms

    if let Ok(mut file) = File::create("analysis_context.txt") {
        file.write_all(serde_json::to_string(&context).unwrap().as_bytes()).unwrap()
    }

    /* fixme
    let stats = context.stats();
    if let Ok(mut file) = File::create("analysis_stats.txt") {
        file.write_all(serde_json::to_string(&stats).unwrap().as_bytes()).unwrap()
    }
    */

    #[cfg(debug_assertions)]
    println!("{}", print_hierarchy(&context, 0));

    #[cfg(debug_assertions)]
    {
        let mut map = HashMap::<Language, u32>::new();
        count_extensions(&context, &mut map);
        println!("Files: {:#?}", map);
    }

    #[cfg(debug_assertions)]
    {
        println!("The max amount of memory that was used {}GB", PEAK_ALLOC.peak_usage_as_gb());
        println!("The currently used amount of memory is {}GB", PEAK_ALLOC.current_usage_as_gb());
    }
}

fn scan_dir(dir: PathBuf) -> CountContext {
    let dir_name = dir.file_name().unwrap().to_str().unwrap().to_string();
    let entry_list = fs::read_dir(&dir);
    if entry_list.is_err() {
        eprintln!("Couldn't scan {}: {} - Skipping...", dir.display(), entry_list.err().unwrap());
        return CountContext::new(dir_name)
    }
    let entry_list: Vec<PathBuf> = entry_list
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<PathBuf>>();
    let context = entry_list
        .par_iter()
        .filter(|path| {
            if path.ends_with(".repo")
                || path.ends_with(".git")
                || path.ends_with("prebuilt")
                || path.ends_with("prebuilts")
                || path.ends_with("out") {
                return false;
            }
            if path.is_file()
                && ["jar", "so", "obj", "webp", "class", "jpeg", "exe", "webm",
                "mp4", "apk", "apex", "ko", "lz4", "gz", "debug", "cr2",
            ].iter().any(|ext| path.ends_with(ext)) {
                return false;
            }
            if path.is_symlink() {
                return false;
            }
            return true;
        })
        .map(|path| {
            // non empty dir
            if path.is_dir() && path.read_dir().is_ok_and(|mut res| res.next().is_some()) {
                Either::Left(scan_dir(path.clone()))
            } else if path.is_file() {
                Either::Right(SourceFile::new(path))
            } else {
                // TODO: remove these from the stack
                Either::Left(CountContext::new(String::new()))
            }
        })
        .reduce(|| {
            Either::Left(CountContext::new(dir_name.clone()))
        }, |a,b| {
            let b = &b;
            let context = a.either(|context_a| {
                // The left and the right value can all be either file in the
                // root context, dir in the root context or a subset of the root
                // context that needs further merging. In any case this function
                // should return the root context.
                let context_a = &context_a;
                b.clone().either(
                    |context_b| {
                        // Unify two contexts
                        if context_a.dir_name == dir_name
                            && context_b.dir_name == dir_name {
                            let mut context_a = context_a.clone();
                            context_a.files.extend(context_b.files);
                            context_a.dirs.extend(context_b.dirs);
                            context_a
                        } else if context_a.dir_name == dir_name {
                            let mut context = context_a.clone();
                            context.insert_context(context_b);
                            context
                        } else if context_b.dir_name == dir_name {
                            let mut context = context_b.clone();
                            context.insert_context(context_a.clone());
                            context
                        } else {
                            let mut context = CountContext::new(dir_name.clone());
                            context.insert_context(context_a.clone());
                            context.insert_context(context_b);
                            context
                        }
                    }, |file_b| {
                        let mut context_a = context_a.clone();
                        if context_a.dir_name == dir_name {
                            if let Some(file_b) = file_b {
                                context_a.files.push(file_b);
                            }
                            context_a
                        } else {
                            let mut context = CountContext::new(dir_name.clone());
                            context.insert_context(context_a);
                            if let Some(file_b) = file_b {
                                context.files.push(file_b);
                            }
                            context
                        }
                    }
                )
            }, |file_a| {
                let file_a = &file_a;
                b.clone().either(
                    |mut context_b| {
                        let file_a = file_a.clone();
                        // TODO: dedupe with context_a and file_b
                        if context_b.dir_name == dir_name {
                            if let Some(file_a) = file_a {
                                context_b.files.push(file_a);
                            }
                            context_b
                        } else {
                            let mut context = CountContext::new(dir_name.clone());
                            context.insert_context(context_b);
                            if let Some(file_a) = file_a {
                                context.files.push(file_a);
                            }
                            context
                        }
                    }, |file_b| {
                        // Add 2 files to new context
                        let file_a = file_a.clone();
                        let mut context = CountContext::new(dir_name.clone());
                        if let Some(file_a) = file_a {
                            context.files.push(file_a);
                        }
                        if let Some(file_b) = file_b {
                            context.files.push(file_b);
                        }
                        context
                    }
                )
            });
            Either::Left(context)
        });

    context.left().unwrap().clone()
}

fn print_hierarchy(context: &CountContext, level: usize) -> String {
    let mut str = context.name() + "\n";
    {
        let mut dirs_iter = context.dirs.iter().peekable();
        while dirs_iter.peek().is_some() {
            let val = print_hierarchy(&dirs_iter.next().unwrap(), level + 1);
            if dirs_iter.peek().is_some()
                || !context.files.is_empty() {
                str += ("│".repeat(level) + "├ " + val.as_str()).as_str();
            } else {
                str += ("│".repeat(level) + "└ " + val.as_str()).as_str();
            }

        }
    }

    {
        let mut file_iter = context.files.iter().peekable();
        while file_iter.peek().is_some() {
            let val = &file_iter.next().unwrap().name();
            if file_iter.peek().is_some() {
                str += ("│".repeat(level) + "├ " + val.as_str() + "\n").as_str();
            } else {
                str += ("│".repeat(level) + "└ " + val.as_str() + "\n").as_str();
            }

        }
    }
    str // TODO: test
}

/// HashMap::<Language, u32>::new()
fn count_extensions(context: &CountContext, map: &mut HashMap::<Language, u32>) {
    for dir in &context.dirs {
        count_extensions(dir, map);
    }
    for file in &context.files {
        let entry = map.entry(file.lang.clone()).or_insert(0);
        *entry += 1;
    }
}

