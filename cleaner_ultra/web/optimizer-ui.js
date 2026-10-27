/* Close the launch dialog before requesting native work, including rejected starts. */
(()=>{window.CleanerUltraOptimizerStart=body=>action(async()=>{
  $('#optimize-dialog').close();
  await api('/api/space/optimize',{...body,confirmed:true});
  await window.CleanerUltraMaintenance?.optimizer();
  toast(body.mode==='trim'?'Windows выполняет ReTrim. Ожидайте завершения.':'Операция запущена внутри Cleaner Ultra');
});})();
