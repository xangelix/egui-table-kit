# Table
total-rows = { $count ->
    [one] 合計 { $count } 行
   *[other] 合計 { $count } 行
}
passing-filter = フィルタ通過
selected = 選択済み

# Table Header
column-options = 列 of オプション
filter-text = テキストフィルタ 🔍
case-insensitive = 大文字・小文字を区別しない
remove-filter = フィルタを解除
new-highlight-filter = 新しいハイライトフィルタ 〽

toggle-sort = 並べ替えの切り替え ↕
current-ascending = (現在: 昇順)
current-descending = (現在: 降順)
regular-expression =
    正規表現
    無効な正規表現の場合、フィルタリングは行われません。

# Operations
operation-pending = この操作は現在保留中です。
operation-at-least-one = この操作を行うには、少なくとも1行を選択する必要があります。
operation-one = この操作を行うには、正確に1行を選択する必要があります。
operation-at-least-one-filtered = この操作を行うには、テーブルをフィルタリングする必要があります。

error = エラー:
cancel = キャンセル
close = 閉じる

copy-hovered-rows = カーソル下の行をコピー
copy-rows = 行をコピー

copy-hovered-rows-with-headers = カーソル下の行をコピー（ヘッダー付き）
copy-rows-with-headers = 行をコピー（ヘッダー付き）
select-filtered = フィルタ結果を選択
deselect-filtered = フィルタ結果の選択を解除

select-all = すべて選択
deselect-all = すべて選択解除
