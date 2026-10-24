using System.Diagnostics.CodeAnalysis;
using System.Reflection;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

/// <summary>The compilation options of C# method editing, shared by the browser compiler
/// and the native fixture harness.</summary>
public static class CompilerOptions
{
    [UnconditionalSuppressMessage("Trimming", "IL2026", Justification = "Roslyn is a trimming root.")]
    [UnconditionalSuppressMessage("Trimming", "IL2075", Justification = "Roslyn is a trimming root.")]
    public static CSharpCompilationOptions Create()
    {
        var options = new CSharpCompilationOptions(
            OutputKind.DynamicallyLinkedLibrary,
            optimizationLevel: OptimizationLevel.Release,
            allowUnsafe: true,
            nullableContextOptions: NullableContextOptions.Disable,
            metadataImportOptions: MetadataImportOptions.All);
        // As dnSpy does: the edited method may use the private and internal members of
        // the assembly it belongs to (Roslyn's internal BinderFlags.IgnoreAccessibility).
        var flags = typeof(CSharpCompilationOptions).Assembly.GetType("Microsoft.CodeAnalysis.CSharp.BinderFlags")!;
        typeof(CSharpCompilationOptions)
            .GetProperty("TopLevelBinderFlags", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(options, Enum.ToObject(flags, 1u << 22));
        return options;
    }
}
