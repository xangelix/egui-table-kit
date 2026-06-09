# Table
total-rows = { $count ->
    [one] { $count } Zeile insgesamt
   *[other] { $count } Zeilen insgesamt
}
passing-filter = im Filter
selected = ausgewählt

# Table Header
column-options = Spaltenoptionen
filter-text = Filtertext 🔍
case-insensitive = Groß-/Kleinschreibung ignorieren
remove-filter = Filter entfernen
new-highlight-filter = Neuer Hervorhebungsfilter 〽

toggle-sort = Sortierung umschalten ↕
current-ascending = (Aktuell: Aufsteigend)
current-descending = (Aktuell: Absteigend)
regular-expression =
    Regulärer Ausdruck
    Wenn Regex ungültig ist, wird nichts gefiltert.

# Operations
operation-pending = Dieser Vorgang ist derzeit ausstehend.
operation-at-least-one = Sie müssen mindestens eine Zeile auswählen, um diesen Vorgang zu nutzen.
operation-one = Sie müssen genau eine Zeile auswählen, um diesen Vorgang zu nutzen.
operation-at-least-one-filtered = Sie müssen die Tabelle filtern, um diesen Vorgang zu nutzen.

error = Fehler:
cancel = Abbrechen
close = Schließen

copy-hovered-rows = Zeilen unter Mauszeiger kopieren
copy-rows = Zeilen kopieren

copy-hovered-rows-with-headers = Zeilen unter Mauszeiger kopieren (mit Kopfzeilen)
copy-rows-with-headers = Zeilen kopieren (mit Kopfzeilen)
select-filtered = Gefilterte auswählen
deselect-filtered = Gefilterte abwählen

select-all = Alle auswählen
deselect-all = Alle abwählen
