#!/usr/bin/env nu

const rm_ro_dir = path self ../scripts/rm-ro-dir.sh
const check_tar_lz = path self ../scripts/check-tar-lz.sh

export def compress_dir [base: string, output: string] {
  let list = ls -l $base | sort-by created | each {|p| $p.name } | prepend $base | str join (char nul)
  $list | 
    tar -c --posix --mode="a-w" --sort none --no-recursion --null --files-from - | 
    dab-util-firefox-backup tar-to-cbor |
    lzip -9 --dictionary-size=64MiB -o $output
}

export def main [source: string, dest: string, year: int, month: int] {
  let month_str = $month | into string | fill --alignment right --character '0' --width 2
  let src_dir = $"($year)-($month_str)"
  let dest = [$dest, $"($year)/($year)-($month_str).tar.lz" ] | path join | path expand
  cd $source
  compress_dir $src_dir $dest
  bash $check_tar_lz $dest
  bash $rm_ro_dir $src_dir
}
