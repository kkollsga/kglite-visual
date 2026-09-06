/**
 * External consumer check using an explicitly supplied Gephi toolkit classpath.
 * Compile with javac -cp <gephi-toolkit-all.jar> -d target/gephi-consumer this-file.
 * Run GephiImportProbe <export-file> <nodes> <edges> <self-loops> on that classpath.
 * Toolkit/runtime and compiled classes belong under target, never beside this source.
 * This hand-authored probe invokes public importer APIs; it contains no Gephi implementation.
 */
import java.io.File;
import java.util.TreeMap;
import org.openide.util.Lookup;
import org.gephi.io.importer.api.*;
import org.gephi.graph.api.*;

public class GephiImportProbe {
    public static void main(String[] args) throws Exception {
        if (args.length != 4) throw new IllegalArgumentException("file nodes edges self-loops");
        var importer = Lookup.getDefault().lookup(ImportController.class);
        var container = importer.importFile(new File(args[0]));
        container.getLoader().setAllowSelfLoop(true);
        container.getLoader().setAllowParallelEdge(true);
        container.getLoader().setEdgesMergeStrategy(EdgeMergeStrategy.NO_MERGE);
        if (!container.verify()) throw new AssertionError("Gephi importer verification failed");
        var workspace = importer.process(container);
        var graph = Lookup.getDefault().lookup(GraphController.class).getGraphModel(workspace).getGraph();
        int loops = 0;
        var pairs = new TreeMap<String,Integer>();
        for (var edge : graph.getEdges()) {
            if (edge.isSelfLoop()) loops++;
            if (!edge.isDirected()) throw new AssertionError("Direction lost on " + edge.getId());
            pairs.merge(edge.getSource().getId() + "->" + edge.getTarget().getId(), 1, Integer::sum);
        }
        if (graph.getNodeCount() != Integer.parseInt(args[1]) || graph.getEdgeCount() != Integer.parseInt(args[2]) || loops != Integer.parseInt(args[3]))
            throw new AssertionError("counts " + graph.getNodeCount() + "/" + graph.getEdgeCount() + "/" + loops);
        var labels = new TreeMap<String,String>();
        for (var node : graph.getNodes()) labels.put(node.getId().toString(), node.getLabel());
        System.out.println("Gephi imported " + args[0]);
        System.out.println("nodes=" + graph.getNodeCount() + " edges=" + graph.getEdgeCount() + " self-loops=" + loops);
        System.out.println("labels=" + labels);
        System.out.println("directed endpoint multiplicities=" + pairs);
    }
}
