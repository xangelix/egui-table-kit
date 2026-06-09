# Table
total-rows = { $count ->
    [one] { $count } row total
   *[other] { $count } rows total
}
passing-filter = passing filter
selected = selected

# Table Header
column-options = Column Options
filter-text = Filter Text 🔍
case-insensitive = Case Insensitive
remove-filter = Remove Filter
new-highlight-filter = New Highlight Filter 〽

toggle-sort = Toggle Sort ↕
current-ascending = (Current: Ascending)
current-descending = (Current: Descending)
regular-expression =
    Regular Expression
    If invalid regex, this will filter nothing.

# Operations
operation-pending = This operation is currently pending.
operation-at-least-one = You must select at least one row to use this operation.
operation-one = You must select exactly one row to use this operation.
operation-at-least-one-filtered = You must filter the table to use this operation.

error = Error:
cancel = Cancel
close = Close

copy-hovered-rows = Copy Hovered Rows
copy-rows = Copy Rows

copy-hovered-rows-with-headers = Copy Hovered Rows (with Headers)
copy-rows-with-headers = Copy Rows (with Headers)
select-filtered = Select Filtered
deselect-filtered = Deselect Filtered

select-all = Select All
deselect-all = Deselect All
