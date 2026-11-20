//! The fixed comparison set: two generated translation-unit corpora, pinned
//! zlib and a pinned LLVM target subset. Each workload says how to prepare a
//! tree for a compiler command (untimed), the build the timer measures, how to
//! clean its outputs, which files its edits touch, and which outputs it makes.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// How a build reaches the compiler: the command a build system runs as its C
/// (or C++) compiler, flags kache would add included, and the environment.
#[derive(Debug, Clone)]
pub struct CompilerCmd {
    /// `CC` for make-driven builds, e.g. `ccache /usr/bin/gcc -ffile-prefix-map=...`.
    pub cc: String,
    pub cxx: String,
    /// The launcher alone (`kache` / `ccache`), for CMake.
    pub launcher: Option<PathBuf>,
    pub cc_path: PathBuf,
    pub cxx_path: PathBuf,
    /// Flags kache adds by default, given to raw and ccache explicitly.
    pub aligned_flags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    CorpusC,
    CorpusCxx,
    Zlib,
    Llvm,
}

#[derive(Debug, Clone)]
pub struct Workload {
    pub name: &'static str,
    pub kind: Kind,
    /// Translation units the build compiles, for per-unit times.
    pub units: usize,
    pub relocatable: bool,
}

pub const C_UNITS: usize = 48;
pub const CXX_UNITS: usize = 12;

pub fn all() -> Vec<Workload> {
    vec![
        Workload { name: "corpus-c-tiny", kind: Kind::CorpusC, units: C_UNITS, relocatable: true },
        Workload { name: "corpus-cxx-headers", kind: Kind::CorpusCxx, units: CXX_UNITS, relocatable: true },
        Workload { name: "zlib", kind: Kind::Zlib, units: 0, relocatable: true },
        Workload { name: "llvm-subset", kind: Kind::Llvm, units: 0, relocatable: false },
    ]
}

/// Pinned inputs, verified against the lock before any timing.
#[derive(Debug, Clone, Default)]
pub struct Archives {
    pub zlib: Option<PathBuf>,
    pub llvm: Option<PathBuf>,
}

pub const LLVM_TARGETS: [&str; 2] = ["llvm-tblgen", "LLVMCore"];

impl Workload {
    /// Write or unpack the sources and configure the build for `compiler`.
    pub fn prepare(&self, tree: &Path, compiler: &CompilerCmd, archives: &Archives, env: &[(String, String)], jobs: usize) -> Result<()> {
        if tree.exists() {
            std::fs::remove_dir_all(tree)?;
        }
        std::fs::create_dir_all(tree)?;
        match self.kind {
            Kind::CorpusC => write_corpus_c(tree),
            Kind::CorpusCxx => write_corpus_cxx(tree),
            Kind::Zlib => {
                let archive = archives.zlib.as_ref().context("zlib needs --zlib-archive")?;
                run(Command::new("tar").arg("xzf").arg(archive).arg("--strip-components=1").arg("-C").arg(tree), env)?;
                run(
                    Command::new("./configure").arg("--static").current_dir(tree).env("CC", &compiler.cc),
                    env,
                )
            }
            Kind::Llvm => {
                let archive = archives.llvm.as_ref().context("llvm-subset needs --llvm-archive")?;
                let stem = "llvm-project-21.1.8.src";
                run(
                    Command::new("tar")
                        .arg("xJf")
                        .arg(archive)
                        .arg("--strip-components=1")
                        .arg("-C")
                        .arg(tree)
                        .args([format!("{stem}/llvm"), format!("{stem}/cmake"), format!("{stem}/third-party")]),
                    env,
                )?;
                let flags = compiler.aligned_flags.join(" ");
                let mut cmake = Command::new("cmake");
                cmake
                    .arg("-S")
                    .arg(tree.join("llvm"))
                    .arg("-B")
                    .arg(tree.join("build"))
                    .args(["-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DLLVM_TARGETS_TO_BUILD=X86"])
                    .args(["-DLLVM_ENABLE_ASSERTIONS=OFF", "-DLLVM_APPEND_VC_REV=OFF", "-DLLVM_INCLUDE_TESTS=OFF"])
                    .args(["-DLLVM_INCLUDE_BENCHMARKS=OFF", "-DLLVM_INCLUDE_EXAMPLES=OFF"])
                    .args(["-DLLVM_ENABLE_ZLIB=OFF", "-DLLVM_ENABLE_ZSTD=OFF", "-DLLVM_ENABLE_LIBXML2=OFF"])
                    .arg(format!("-DLLVM_PARALLEL_COMPILE_JOBS={jobs}"))
                    .arg(format!("-DCMAKE_C_COMPILER={}", compiler.cc_path.display()))
                    .arg(format!("-DCMAKE_CXX_COMPILER={}", compiler.cxx_path.display()))
                    .arg(format!("-DCMAKE_C_FLAGS={flags}"))
                    .arg(format!("-DCMAKE_CXX_FLAGS={flags}"));
                if let Some(launcher) = &compiler.launcher {
                    cmake
                        .arg(format!("-DCMAKE_C_COMPILER_LAUNCHER={}", launcher.display()))
                        .arg(format!("-DCMAKE_CXX_COMPILER_LAUNCHER={}", launcher.display()));
                }
                run(&mut cmake, env)
            }
        }
    }

    /// The build the timer measures.
    pub fn build(&self, tree: &Path, compiler: &CompilerCmd, jobs: usize) -> Command {
        match self.kind {
            Kind::CorpusC | Kind::CorpusCxx => {
                let mut make = Command::new("make");
                let jobs = if self.kind == Kind::CorpusC { 1 } else { jobs };
                make.arg(format!("-j{jobs}"))
                    .arg(format!("CC={}", compiler.cc))
                    .arg(format!("CXX={}", compiler.cxx))
                    .current_dir(tree);
                make
            }
            Kind::Zlib => {
                let mut make = Command::new("make");
                make.arg(format!("-j{jobs}")).args(["libz.a", "example", "minigzip"]).current_dir(tree);
                make
            }
            Kind::Llvm => {
                let mut ninja = Command::new("ninja");
                ninja.arg(format!("-j{jobs}")).args(LLVM_TARGETS).current_dir(tree.join("build"));
                ninja
            }
        }
    }

    /// One unit's compile alone, for the daemon start-up measurement.
    pub fn single_unit(&self, tree: &Path, compiler: &CompilerCmd) -> Option<(Command, PathBuf)> {
        (self.kind == Kind::CorpusC).then(|| {
            let mut make = Command::new("make");
            make.arg(format!("CC={}", compiler.cc)).arg("t00.o").current_dir(tree);
            (make, tree.join("t00.o"))
        })
    }

    /// Remove the build's outputs, keeping its configuration.
    pub fn clean(&self, tree: &Path, env: &[(String, String)]) -> Result<()> {
        match self.kind {
            Kind::CorpusC | Kind::CorpusCxx | Kind::Zlib => {
                for path in outputs(tree)? {
                    std::fs::remove_file(path)?;
                }
                for name in ["libz.a", "example", "minigzip"] {
                    let _ = std::fs::remove_file(tree.join(name));
                }
                Ok(())
            }
            Kind::Llvm => run(Command::new("ninja").args(["-t", "clean"]).current_dir(tree.join("build")), env),
        }
    }

    /// The file a source edit changes and the one a header edit changes.
    pub fn edit_targets(&self) -> (&'static str, &'static str) {
        match self.kind {
            Kind::CorpusC => ("t07.c", "common.h"),
            Kind::CorpusCxx => ("x03.cpp", "common.hpp"),
            Kind::Zlib => ("adler32.c", "zutil.h"),
            Kind::Llvm => ("llvm/lib/IR/Comdat.cpp", "llvm/include/llvm/IR/Comdat.h"),
        }
    }

    pub fn outputs(&self, tree: &Path) -> Result<Vec<PathBuf>> {
        outputs(tree)
    }
}

/// A declaration appended to a file: a real change to every unit reading it.
/// Each `tag` names its own declaration, so two edits are two new states.
pub fn edited(original: &[u8], tag: &str) -> Vec<u8> {
    let mut bytes = original.to_vec();
    bytes.extend_from_slice(format!("\nextern int kache_bench_edit_{tag};\n").as_bytes());
    bytes
}

/// Every object file under `tree`, sorted.
fn outputs(tree: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![tree.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                if !path.ends_with("CMakeFiles/CMakeScratch") {
                    stack.push(path);
                }
            } else if kind.is_file() && path.extension().is_some_and(|e| e == "o") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Run `command` with exactly `env` plus the variables it set itself.
fn run(command: &mut Command, env: &[(String, String)]) -> Result<()> {
    let own: Vec<(std::ffi::OsString, std::ffi::OsString)> =
        command.get_envs().filter_map(|(k, v)| v.map(|v| (k.to_owned(), v.to_owned()))).collect();
    command.env_clear().envs(env.iter().map(|(k, v)| (k, v))).envs(own);
    let output = command.output().with_context(|| format!("running {command:?}"))?;
    if !output.status.success() {
        bail!(
            "{command:?} failed: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
        );
    }
    Ok(())
}

fn write_corpus_c(tree: &Path) -> Result<()> {
    std::fs::write(tree.join("common.h"), "#define COMMON_SCALE 3\nint common_bias(void);\n")?;
    let mut objects = Vec::new();
    for i in 0..C_UNITS {
        std::fs::write(
            tree.join(format!("t{i:02}.c")),
            format!(
                "#include \"common.h\"\nint t{i:02}(int x) {{\n  int acc = {i};\n  for (int k = 0; k < x; k++)\n    acc = acc * COMMON_SCALE + k;\n  return acc;\n}}\n"
            ),
        )?;
        objects.push(format!("t{i:02}.o"));
    }
    std::fs::write(
        tree.join("Makefile"),
        format!(
            "OBJS = {}\nall: $(OBJS)\n%.o: %.c common.h\n\t$(CC) -O2 -c $< -o $@\n",
            objects.join(" ")
        ),
    )?;
    Ok(())
}

fn write_corpus_cxx(tree: &Path) -> Result<()> {
    std::fs::write(
        tree.join("common.hpp"),
        "#include <algorithm>\n#include <functional>\n#include <iostream>\n#include <map>\n#include <memory>\n#include <optional>\n#include <regex>\n#include <sstream>\n#include <string>\n#include <unordered_map>\n#include <variant>\n#include <vector>\n",
    )?;
    let mut objects = Vec::new();
    for i in 0..CXX_UNITS {
        std::fs::write(
            tree.join(format!("x{i:02}.cpp")),
            format!(
                "#include \"common.hpp\"\nstd::string x{i:02}(const std::vector<std::string>& in) {{\n  std::map<std::string, int> counts;\n  for (const auto& s : in) counts[s] += {i};\n  std::ostringstream out;\n  std::regex word(\"[a-z]+\");\n  for (const auto& [k, v] : counts)\n    if (std::regex_match(k, word)) out << k << v;\n  return out.str();\n}}\n"
            ),
        )?;
        objects.push(format!("x{i:02}.o"));
    }
    std::fs::write(
        tree.join("Makefile"),
        format!(
            "OBJS = {}\nall: $(OBJS)\n%.o: %.cpp common.hpp\n\t$(CXX) -std=c++17 -O1 -c $< -o $@\n",
            objects.join(" ")
        ),
    )?;
    Ok(())
}
