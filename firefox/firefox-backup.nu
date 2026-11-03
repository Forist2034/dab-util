export def compress_dir [base: string, output: string] {
  let list = ls -l $base | sort-by created | each {|p| $p.name } | prepend $base | str join (char nul)
  $list | 
    tar -c --posix --mode="a-w" --sort none --no-recursion --null --files-from - | 
    dab-util-firefox-backup tar-to-cbor |
    lzip -9 --dictionary-size=64MiB -o $output
}
