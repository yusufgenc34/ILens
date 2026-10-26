// Owned, synthetic packer shape: the real code is a managed assembly carried in an
// embedded resource. ILens reads that resource as bytes; nothing here is ever run.
namespace ILens.Packed;
public static class Loader
{
    public const string Payload = "ILens.Packed.Payload.dll";
    public static int Size => Payload.Length;
    // Deliberately the same literal as ILens.Deobfuscation uses, so "where is this
    // string used" can be checked across two assemblies in one workspace.
    public static string Shared() => "fixture-do-not-open.txt";
}
