// ILens's C# compiler component: Roslyn compiled to WebAssembly, loaded only when a
// method is edited in C#. It compiles source against reference assemblies given as
// bytes (read as metadata, never executed) and returns diagnostics and the PE image.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices.JavaScript;
using System.Runtime.Versioning;
using System.Text.Json;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

return;

[SupportedOSPlatform("browser")]
public static partial class Compiler
{
    static readonly List<MetadataReference> references = new();
    static byte[] output = Array.Empty<byte>();

    [JSExport]
    public static void Reset()
    {
        references.Clear();
        output = Array.Empty<byte>();
    }

    [JSExport]
    public static void AddReference(byte[] image, string name) =>
        references.Add(MetadataReference.CreateFromImage(image, filePath: name));

    /// <summary>Compiles one source file; returns {success, diagnostics} as JSON.</summary>
    [JSExport]
    public static string Compile(string source, string assemblyName)
    {
        var options = CompilerOptions.Create();
        var tree = CSharpSyntaxTree.ParseText(source, new CSharpParseOptions(LanguageVersion.Latest));
        var compilation = CSharpCompilation.Create(assemblyName, new[] { tree }, references, options);
        using var stream = new MemoryStream();
        var result = compilation.Emit(stream);
        output = result.Success ? stream.ToArray() : Array.Empty<byte>();
        var diagnostics = result.Diagnostics
            .Where(d => d.Severity == DiagnosticSeverity.Error)
            .Take(200)
            .Select(d =>
            {
                var span = d.Location.GetLineSpan();
                return new Diagnostic(d.Id, d.GetMessage(), span.StartLinePosition.Line + 1, span.StartLinePosition.Character + 1);
            });
        return JsonSerializer.Serialize(new Result(result.Success, diagnostics.ToArray()), ResultContext.Default.Result);
    }

    [JSExport]
    public static byte[] Output() => output;
}
public record Diagnostic(string Id, string Message, int Line, int Column);
public record Result(bool Success, Diagnostic[] Diagnostics);
[System.Text.Json.Serialization.JsonSerializable(typeof(Result))]
public partial class ResultContext : System.Text.Json.Serialization.JsonSerializerContext { }
