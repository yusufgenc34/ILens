# dnSpy project export reference

The archived dnSpy export implementation was inspected to correct ILens's initial export workflow. An IL/metadata archive with an optional generic project file did not provide the expected project-export behavior.

## Inspected paths

- [ExportToProjectDlg.xaml](https://github.com/dnSpy/dnSpy/blob/master/dnSpy/dnSpy/Documents/Tabs/Dialogs/ExportToProjectDlg.xaml) and its [view model](https://github.com/dnSpy/dnSpy/blob/master/dnSpy/dnSpy/Documents/Tabs/Dialogs/ExportToProjectVM.cs) expose solution creation, destination, language and resource conversion options, progress and errors.
- [MSBuildProjectCreator.cs](https://github.com/dnSpy/dnSpy/blob/master/dnSpy/dnSpy.Decompiler/MSBuild/MSBuildProjectCreator.cs) creates a project for each selected module, performs file jobs, writes project files and optionally writes a solution.
- [Project.cs](https://github.com/dnSpy/dnSpy/blob/master/dnSpy/dnSpy.Decompiler/MSBuild/Project.cs) schedules type sources, assembly information, resources and supported designer/native-resource work.
- [FilenameCreator.cs](https://github.com/dnSpy/dnSpy/blob/master/dnSpy/dnSpy.Decompiler/MSBuild/FilenameCreator.cs) organizes namespace paths relative to the default namespace and disambiguates filenames.
- [ProjectWriter.cs](https://github.com/dnSpy/dnSpy/blob/master/dnSpy/dnSpy.Decompiler/MSBuild/ProjectWriter.cs) emits build properties and distinguishes assembly references from references to other exported projects.
- [SolutionWriter.cs](https://github.com/dnSpy/dnSpy/blob/master/dnSpy/dnSpy.Decompiler/MSBuild/SolutionWriter.cs) groups projects in a Visual Studio solution with build configurations.

## ILens behavior

**File > Export to Project** selects loaded assemblies and creates a ZIP containing a solution and individual projects. Class files use familiar filenames; namespace folders are relative to the common namespace. Nested types remain in the declaring type's source. Each project has its own assembly name, target framework, PE-derived output/platform settings, supported startup type, version/culture attributes, logical resource names and identity-resolved references.

Selecting both sides of a resolved reference produces `ProjectReference`. Other explicitly loaded dependencies can be copied to `References`; unavailable references retain their identity without guessed NuGet packages or downloads. Diagnostic files live in `_ilens`, with independent IL retained for unsupported methods. **Save Code** and **Save Module** are separate actions in the File menu. **Edit IL** is available only in the code viewer's IL tab.

The browser uses a ZIP download in place of dnSpy's desktop destination-folder write. ILens exports C# SDK-style projects; it does not advertise unimplemented language/Visual Studio version controls. Full attribute reconstruction, BAML/XAML, resx conversion, WPF/WinForms designers, application manifests/icons, signing and general compilable-project parity remain limitations. A generated solution does not imply that every method was reconstructed successfully.

This is a workflow and file-format reference. No dnSpy source code was copied, translated or embedded. Its managed runtime, filesystem access and execution/debugging features are not included in ILens. The Rust static-analysis engine remains independent.
