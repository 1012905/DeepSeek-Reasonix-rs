// wiremanifest generates crates/contract/src/eventwire_manifest.json from the
// Go wire types via reflection, so the manifest cannot drift from the kernel
// side. Both parity tests (Rust and TS) read it as the field-set authority.
//
// Run from the module root: go run ./tools/wiremanifest
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"sort"
	"strings"

	"reasonix/internal/contract/agentgraph"
	"reasonix/internal/contract/delivery"
	"reasonix/internal/contract/event"
	"reasonix/internal/contract/eventwire"
	"reasonix/internal/contract/pricing"
	"reasonix/internal/contract/provider"
	"reasonix/internal/contract/surfacespec"
)

const defaultOut = "crates/contract/src/eventwire_manifest.json"

func main() {
	out := defaultOut
	if len(os.Args) > 1 {
		out = os.Args[1]
	}

	roots := map[string]reflect.Type{
		"eventwire.Event":                 reflect.TypeOf(eventwire.Event{}),
		"eventwire.Approval":              reflect.TypeOf(eventwire.Approval{}),
		"eventwire.RecoveryApproval":      reflect.TypeOf(eventwire.RecoveryApproval{}),
		"eventwire.Ask":                   reflect.TypeOf(eventwire.Ask{}),
		"eventwire.AskQuestion":           reflect.TypeOf(eventwire.AskQuestion{}),
		"eventwire.AskOption":             reflect.TypeOf(eventwire.AskOption{}),
		"eventwire.AskOrigin":             reflect.TypeOf(eventwire.AskOrigin{}),
		"eventwire.Attachment":            reflect.TypeOf(eventwire.Attachment{}),
		"eventwire.Bound":                 reflect.TypeOf(eventwire.Bound{}),
		"eventwire.CacheDiagnostics":      reflect.TypeOf(eventwire.CacheDiagnostics{}),
		"eventwire.Compaction":            reflect.TypeOf(eventwire.Compaction{}),
		"eventwire.CompletionReceipt":     reflect.TypeOf(eventwire.CompletionReceipt{}),
		"eventwire.CompletionSummary":     reflect.TypeOf(eventwire.CompletionSummary{}),
		"eventwire.ContextMaintenance":    reflect.TypeOf(eventwire.ContextMaintenance{}),
		"eventwire.DecisionReceipt":       reflect.TypeOf(eventwire.DecisionReceipt{}),
		"eventwire.ExtensionActionRef":    reflect.TypeOf(eventwire.ExtensionActionRef{}),
		"eventwire.ExtensionCard":         reflect.TypeOf(eventwire.ExtensionCard{}),
		"eventwire.ExtensionFormField":    reflect.TypeOf(eventwire.ExtensionFormField{}),
		"eventwire.ExtensionForm":         reflect.TypeOf(eventwire.ExtensionForm{}),
		"eventwire.ExtensionKeyValue":     reflect.TypeOf(eventwire.ExtensionKeyValue{}),
		"eventwire.ExtensionNotification": reflect.TypeOf(eventwire.ExtensionNotification{}),
		"eventwire.ExtensionPanel":        reflect.TypeOf(eventwire.ExtensionPanel{}),
		"eventwire.ExtensionStatus":       reflect.TypeOf(eventwire.ExtensionStatus{}),
		"eventwire.ExtensionSurface":      reflect.TypeOf(eventwire.ExtensionSurface{}),
		"eventwire.ExtensionView":         reflect.TypeOf(eventwire.ExtensionView{}),
		"eventwire.ExtensionViewNode":     reflect.TypeOf(eventwire.ExtensionViewNode{}),
		"eventwire.Guardian":              reflect.TypeOf(eventwire.Guardian{}),
		"eventwire.MemoryCitation":        reflect.TypeOf(eventwire.MemoryCitation{}),
		"eventwire.Profile":               reflect.TypeOf(eventwire.Profile{}),
		"eventwire.ProgressWatch":         reflect.TypeOf(eventwire.ProgressWatch{}),
		"eventwire.ReceiptChange":         reflect.TypeOf(eventwire.ReceiptChange{}),
		"eventwire.ReceiptGap":            reflect.TypeOf(eventwire.ReceiptGap{}),
		"eventwire.ReceiptVerification":   reflect.TypeOf(eventwire.ReceiptVerification{}),
		"eventwire.ShellExecution":        reflect.TypeOf(eventwire.ShellExecution{}),
		"eventwire.StreamAttempt":         reflect.TypeOf(eventwire.StreamAttempt{}),
		"eventwire.Tool":                  reflect.TypeOf(eventwire.Tool{}),
		"eventwire.TodoProgress":          reflect.TypeOf(eventwire.TodoProgress{}),
		"eventwire.Usage":                 reflect.TypeOf(eventwire.Usage{}),
		"eventwire.Via":                   reflect.TypeOf(eventwire.Via{}),
		"eventwire.VideoShown":            reflect.TypeOf(eventwire.VideoShown{}),
		"eventwire.WorkspaceChanged":      reflect.TypeOf(eventwire.WorkspaceChanged{}),
		"eventwire.WorkspaceLease":        reflect.TypeOf(eventwire.WorkspaceLease{}),
		"eventwire.WorkspacePathChange":   reflect.TypeOf(eventwire.WorkspacePathChange{}),
		"eventwire.WorkspaceRevision":     reflect.TypeOf(eventwire.WorkspaceRevision{}),
		"event.ContextComposition":        reflect.TypeOf(event.ContextComposition{}),
		"event.ContextItem":               reflect.TypeOf(event.ContextItem{}),
		"event.ContextPart":               reflect.TypeOf(event.ContextPart{}),
		"delivery.File":                   reflect.TypeOf(delivery.File{}),
		"pricing.CostQuote":               reflect.TypeOf(pricing.CostQuote{}),
		"pricing.Money":                   reflect.TypeOf(pricing.Money{}),
		"pricing.RateSnapshot":            reflect.TypeOf(pricing.RateSnapshot{}),
		"pricing.Valuation":               reflect.TypeOf(pricing.Valuation{}),
		"agentgraph.Node":                 reflect.TypeOf(agentgraph.Node{}),
		"agentgraph.Edge":                 reflect.TypeOf(agentgraph.Edge{}),
		"agentgraph.Delta":                reflect.TypeOf(agentgraph.Delta{}),
		"agentgraph.Graph":                reflect.TypeOf(agentgraph.Graph{}),
		"surfacespec.Spec":                reflect.TypeOf(surfacespec.Spec{}),
		"surfacespec.Node":                reflect.TypeOf(surfacespec.Node{}),
		"surfacespec.Field":               reflect.TypeOf(surfacespec.Field{}),
		"surfacespec.Option":              reflect.TypeOf(surfacespec.Option{}),
		"surfacespec.Diff":                reflect.TypeOf(surfacespec.Diff{}),
		"surfacespec.Image":               reflect.TypeOf(surfacespec.Image{}),
		"surfacespec.Location":            reflect.TypeOf(surfacespec.Location{}),
		"surfacespec.Fact":                reflect.TypeOf(surfacespec.Fact{}),
		"surfacespec.Dataset":             reflect.TypeOf(surfacespec.Dataset{}),
		"surfacespec.Series":              reflect.TypeOf(surfacespec.Series{}),
		"provider.ArgumentNormalization":  reflect.TypeOf(provider.ArgumentNormalization{}),
	}

	fields := map[string]map[string]bool{}
	for name, t := range roots {
		m := map[string]bool{}
		collect(t, m)
		fields[name] = m
	}

	enc := json.NewEncoder(nil)
	_ = enc
	data, err := marshalManifest(eventwire.KindNames(), fields)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := os.WriteFile(out, data, 0o644); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Fprintf(os.Stderr, "wrote %s (%d types)\n", out, len(fields))
}

// collect lists the JSON field names one wire type emits — its own fields
// only; nested structs are manifest entries of their own and never walked
// into. The value records whether the field is omitempty, which decides the
// required set a default-valued instance must serialize.
func collect(t reflect.Type, into map[string]bool) {
	for t.Kind() == reflect.Pointer || t.Kind() == reflect.Slice || t.Kind() == reflect.Array || t.Kind() == reflect.Map {
		t = t.Elem()
	}
	if t.Kind() != reflect.Struct || t == reflect.TypeOf(json.RawMessage{}) {
		return
	}
	for i := 0; i < t.NumField(); i++ {
		f := t.Field(i)
		if !f.IsExported() {
			continue
		}
		parts := strings.Split(f.Tag.Get("json"), ",")
		name := parts[0]
		if name == "-" {
			continue
		}
		if name == "" {
			name = f.Name
		}
		omit := false
		for _, opt := range parts[1:] {
			if opt == "omitempty" {
				omit = true
				break
			}
		}
		into[name] = omit
	}
}

// marshalManifest builds deterministic JSON bytes: every map is emitted
// through sorted key lists so the manifest diffs cleanly across regenerations.
// Per type, `fields` is the whole wire field set and `omitted` the omitempty
// subset — everything else must appear in every encoding.
func marshalManifest(kinds []string, fields map[string]map[string]bool) ([]byte, error) {
	typeNames := make([]string, 0, len(fields))
	for name := range fields {
		typeNames = append(typeNames, name)
	}
	sort.Strings(typeNames)

	types := make(map[string]any, len(fields))
	for _, name := range typeNames {
		all := make([]string, 0, len(fields[name]))
		omitted := make([]string, 0)
		for f, omit := range fields[name] {
			all = append(all, f)
			if omit {
				omitted = append(omitted, f)
			}
		}
		sort.Strings(all)
		sort.Strings(omitted)
		types[name] = map[string]any{"fields": all, "omitted": omitted}
	}
	sortedKinds := append([]string(nil), kinds...)
	sort.Strings(sortedKinds)

	out := map[string]any{"kinds": sortedKinds, "types": types}
	b, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(b, '\n'), nil
}
