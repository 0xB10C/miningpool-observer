#!/bin/bash

PLAIN_SVG_FILES="subpage_missing_transaction
subpage_template_and_block
subpage_block_with_sanctioned_transactions
subpage_block_with_conflicting_transactions
mainpage_templates_and_blocks
mainpage_sanctioned_transactions
mainpage_missing_transactions
mainpage_conflicting_transactions
mainpage_faq
mainpage_index
"

for f in $PLAIN_SVG_FILES
do
    echo "Processing $f"
    scour $f-plain.svg ../../www/templates/svg/$f.svg --enable-viewboxing --enable-id-stripping   --enable-comment-stripping --shorten-ids
    sed -i 's/<?xml version="1.0" encoding="UTF-8"?>//' ../../www/templates/svg/$f.svg
done


