// harness <source.cs> <output.dll> <reference.dll>...
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

var source = File.ReadAllText(args[0]);
var references = args.Skip(2).Select(path => MetadataReference.CreateFromFile(path));
var tree = CSharpSyntaxTree.ParseText(source, new CSharpParseOptions(LanguageVersion.Latest));
var compilation = CSharpCompilation.Create("ILens.CSharpEdit", new[] { tree }, references, CompilerOptions.Create());
using var stream = new MemoryStream();
var result = compilation.Emit(stream);
foreach (var d in result.Diagnostics.Where(d => d.Severity == DiagnosticSeverity.Error))
    Console.Error.WriteLine(d);
if (!result.Success) return 1;
File.WriteAllBytes(args[1], stream.ToArray());
return 0;
