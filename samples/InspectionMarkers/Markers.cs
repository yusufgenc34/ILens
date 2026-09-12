// Detector fixtures only: these attributes imitate public watermark metadata.
// This assembly has NOT been processed by these obfuscators. Never execute it.
using System;
#if MARKERS
[module: ConfusedBy("ConfuserEx fixture marker")]
[assembly: Dotfuscator("fixture marker")]
[assembly: BabelObfuscator("fixture marker")]
[assembly: SmartAssembly.Attributes.PoweredBy("fixture marker")]
#endif

[AttributeUsage(AttributeTargets.Module)]
public sealed class ConfusedByAttribute : Attribute
{
    public ConfusedByAttribute(string version) { }
}
[AttributeUsage(AttributeTargets.Assembly)]
public sealed class DotfuscatorAttribute : Attribute
{
    public DotfuscatorAttribute(string version) { }
}
[AttributeUsage(AttributeTargets.Assembly)]
public sealed class BabelObfuscatorAttribute : Attribute
{
    public BabelObfuscatorAttribute(string version) { }
}
namespace SmartAssembly.Attributes
{
    [AttributeUsage(AttributeTargets.Assembly)]
    public sealed class PoweredByAttribute : Attribute
    {
        public PoweredByAttribute(string version) { }
    }
}

// A configuration attribute and unused marker types are negative controls in
// the NoTarget build. They must not imply that obfuscation was performed.
[System.Reflection.Obfuscation(Exclude = true)]
public static class Example
{
    public static int Calculate(int value) => value + 1;
}
