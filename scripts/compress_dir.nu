#!/usr/bin/env nu

const rm_ro_dir = path self ../scripts/rm-ro-dir.sh
const check_tar_lz = path self ../scripts/check-tar-lz.sh

export def main [source: string, dest: string, year: int, month: int] {
  let month_str = $month | into string | fill --alignment right --character '0' --width 2
  let src_dir = $"($year)-($month_str)"
  let dest = [$dest, $"($year)/($year)-($month_str).tar.lz" ] | path join | path expand
  tar -cv --posix --mode="a-w" --sort name -C $source $src_dir | lzip -9 -o $dest
  bash $check_tar_lz $dest
  bash $rm_ro_dir ($source | path join $src_dir)
}
