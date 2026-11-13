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
	"slices"
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
		"eventwire.Event":                 reflect.TypeFor[eventwire.Event](),
		"eventwire.Approval":              reflect.TypeFor[eventwire.Approval](),
		"eventwire.RecoveryApproval":      reflect.TypeFor[eventwire.RecoveryApproval](),
		"eventwire.Ask":                   reflect.TypeFor[eventwire.Ask](),
		"eventwire.AskQuestion":           reflect.TypeFor[eventwire.AskQuestion](),
		"eventwire.AskOption":             reflect.TypeFor[eventwire.AskOption](),
		"eventwire.AskOrigin":             reflect.TypeFor[eventwire.AskOrigin](),
		"eventwire.Attachment":            reflect.TypeFor[eventwire.Attachment](),
		"eventwire.Bound":                 reflect.TypeFor[eventwire.Bound](),
		"eventwire.CacheDiagnostics":      reflect.TypeFor[eventwire.CacheDiagnostics](),
		"eventwire.Compaction":            reflect.TypeFor[eventwire.Compaction](),
		"eventwire.CompletionReceipt":     reflect.TypeFor[eventwire.CompletionReceipt](),
		"eventwire.CompletionSummary":     reflect.TypeFor[eventwire.CompletionSummary](),
		"eventwire.ContextMaintenance":    reflect.TypeFor[eventwire.ContextMaintenance](),
		"eventwire.DecisionReceipt":       reflect.TypeFor[eventwire.DecisionReceipt](),
		"eventwire.ExtensionActionRef":    reflect.TypeFor[eventwire.ExtensionActionRef](),
		"eventwire.ExtensionCard":         reflect.TypeFor[eventwire.ExtensionCard](),
		"eventwire.ExtensionFormField":    reflect.TypeFor[eventwire.ExtensionFormField](),
		"eventwire.ExtensionForm":         reflect.TypeFor[eventwire.ExtensionForm](),
		"eventwire.ExtensionKeyValue":     reflect.TypeFor[eventwire.ExtensionKeyValue](),
		"eventwire.ExtensionNotification": reflect.TypeFor[eventwire.ExtensionNotification](),
		"eventwire.ExtensionPanel":        reflect.TypeFor[eventwire.ExtensionPanel](),
		"eventwire.ExtensionStatus":       reflect.TypeFor[eventwire.ExtensionStatus](),
		"eventwire.ExtensionSurface":      reflect.TypeFor[eventwire.ExtensionSurface](),
		"eventwire.ExtensionView":         reflect.TypeFor[eventwire.ExtensionView](),
		"eventwire.ExtensionViewNode":     reflect.TypeFor[eventwire.ExtensionViewNode](),
		"eventwire.Guardian":              reflect.TypeFor[eventwire.Guardian](),
		"eventwire.MemoryCitation":        reflect.TypeFor[eventwire.MemoryCitation](),
		"eventwire.Profile":               reflect.TypeFor[eventwire.Profile](),
		"eventwire.ProgressWatch":         reflect.TypeFor[eventwire.ProgressWatch](),
		"eventwire.ReceiptChange":         reflect.TypeFor[eventwire.ReceiptChange](),
		"eventwire.ReceiptGap":            reflect.TypeFor[eventwire.ReceiptGap](),
		"eventwire.ReceiptVerification":   reflect.TypeFor[eventwire.ReceiptVerification](),
		"eventwire.ShellExecution":        reflect.TypeFor[eventwire.ShellExecution](),
		"eventwire.StreamAttempt":         reflect.TypeFor[eventwire.StreamAttempt](),
		"eventwire.Tool":                  reflect.TypeFor[eventwire.Tool](),
		"eventwire.TodoProgress":          reflect.TypeFor[eventwire.TodoProgress](),
		"eventwire.Usage":                 reflect.TypeFor[eventwire.Usage](),
		"eventwire.Via":                   reflect.TypeFor[eventwire.Via](),
		"eventwire.VideoShown":            reflect.TypeFor[eventwire.VideoShown](),
		"eventwire.WorkspaceChanged":      reflect.TypeFor[eventwire.WorkspaceChanged](),
		"eventwire.WorkspaceLease":        reflect.TypeFor[eventwire.WorkspaceLease](),
		"eventwire.WorkspacePathChange":   reflect.TypeFor[eventwire.WorkspacePathChange](),
		"eventwire.WorkspaceRevision":     reflect.TypeFor[eventwire.WorkspaceRevision](),
		"event.ContextComposition":        reflect.TypeFor[event.ContextComposition](),
		"event.ContextItem":               reflect.TypeFor[event.ContextItem](),
		"event.ContextPart":               reflect.TypeFor[event.ContextPart](),
		"delivery.File":                   reflect.TypeFor[delivery.File](),
		"pricing.CostQuote":               reflect.TypeFor[pricing.CostQuote](),
		"pricing.Money":                   reflect.TypeFor[pricing.Money](),
		"pricing.RateSnapshot":            reflect.TypeFor[pricing.RateSnapshot](),
		"pricing.Valuation":               reflect.TypeFor[pricing.Valuation](),
		"agentgraph.Node":                 reflect.TypeFor[agentgraph.Node](),
		"agentgraph.Edge":                 reflect.TypeFor[agentgraph.Edge](),
		"agentgraph.Delta":                reflect.TypeFor[agentgraph.Delta](),
		"agentgraph.Graph":                reflect.TypeFor[agentgraph.Graph](),
		"surfacespec.Spec":                reflect.TypeFor[surfacespec.Spec](),
		"surfacespec.Node":                reflect.TypeFor[surfacespec.Node](),
		"surfacespec.Field":               reflect.TypeFor[surfacespec.Field](),
		"surfacespec.Option":              reflect.TypeFor[surfacespec.Option](),
		"surfacespec.Diff":                reflect.TypeFor[surfacespec.Diff](),
		"surfacespec.Image":               reflect.TypeFor[surfacespec.Image](),
		"surfacespec.Location":            reflect.TypeFor[surfacespec.Location](),
		"surfacespec.Fact":                reflect.TypeFor[surfacespec.Fact](),
		"surfacespec.Dataset":             reflect.TypeFor[surfacespec.Dataset](),
		"surfacespec.Series":              reflect.TypeFor[surfacespec.Series](),
		"provider.ArgumentNormalization":  reflect.TypeFor[provider.ArgumentNormalization](),
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
	if t.Kind() != reflect.Struct || t == reflect.TypeFor[json.RawMessage]() {
		return
	}
	for f := range t.Fields() {
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
		into[name] = slices.Contains(parts[1:], "omitempty")
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
