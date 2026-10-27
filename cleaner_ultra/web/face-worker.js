let model;
onmessage = async ({ data }) => {
  try {
    if (!model) {
      importScripts("/vision/vision_bundle.js");
      const { FaceLandmarker, FilesetResolver } = Vision;
      model = await FaceLandmarker.createFromOptions(
        await FilesetResolver.forVisionTasks("/vision/wasm"),
        {
          baseOptions: {
            modelAssetPath: "/vision/face_landmarker.task",
            delegate: "CPU",
          },
          runningMode: "IMAGE",
          numFaces: 10,
          outputFaceBlendshapes: true,
        },
      );
    }
    const bitmap = await createImageBitmap(
      await (await fetch(`/media/${data.id}?full=1`)).blob(),
    );
    const result = model.detect(bitmap);
    bitmap.close();
    const score = Math.max(
      0,
      ...result.faceBlendshapes.map((face) => {
        const value = (name) =>
          face.categories.find((item) => item.categoryName === name)?.score ||
          0;
        return Math.min(value("eyeBlinkLeft"), value("eyeBlinkRight"));
      }),
    );
    postMessage({ id: data.id, faces: result.faceLandmarks.length, score });
  } catch (error) {
    postMessage({ id: data.id, error: error.message });
  }
};
